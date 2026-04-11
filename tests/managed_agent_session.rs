//! # Session Primitive Contract Tests
//!
//! Validates the **Session** managed-agent primitive against the acceptance bars
//! in `docs/explorations/managed-agents-mapping.md` §1 "Session" and the
//! Anthropic interface:
//!
//! ```text
//! getSession(session_id) → (Session, Event[])
//! getEvents(session_id) → PendingEvent[]
//! emitEvent(id, event)
//! ```
//!
//! *"Any append-only log that can be consumed in order from any event point and
//! accepts idempotent appends — Postgres, SQLite, in-memory array, etc."*
//!
//! Each test function follows the explicit oracle shape: precondition, action,
//! observable evidence, invariant proven. Tests that can verify against current
//! code pass today. Tests that need implementation work are marked `#[ignore]`
//! with a `pending_contract` marker describing the blocker.
//!
//! **Ownership boundary:** this file validates Rust-substrate Session invariants
//! only. External-consumer invariants (browser `StreamDB`, reactive queries,
//! catch-up semantics at the TypeScript layer) belong in `packages/state` and
//! are intentionally out of scope here.

#[path = "support/managed_agent_suite.rs"]
mod managed_agent_suite;

use anyhow::{Context, Result};
use fireline::orchestration::materialize_session_index;
use managed_agent_suite::{
    ControlPlaneHarness, DEFAULT_TIMEOUT, LocalRuntimeHarness, count_events, pending_contract,
    prompt_session, read_session_records, wait_for_event_count, wait_for_session_record,
};
use std::collections::HashSet;

/// Precondition: a local runtime has been spawned and has emitted at least the
/// baseline `session`, `prompt_turn`, and `chunk` envelopes in response to one
/// prompt.
///
/// Action: read the raw durable stream from `Offset::Beginning` to the live
/// edge and collect the body.
///
/// Observable evidence: the returned body is non-empty and contains the expected
/// entity-type envelopes emitted by the runtime's conductor chain.
///
/// Invariant proven: **Session append-only replay from offset 0** — the durable
/// stream exposes every event the harness yielded in the order it was appended,
/// and can be replayed by any authenticated reader from the very beginning.
#[tokio::test]
async fn session_stream_is_append_only_and_replayable_from_beginning() -> Result<()> {
    let runtime = LocalRuntimeHarness::spawn("session-append-only-from-beginning").await?;

    let result = async {
        let _ = runtime
            .prompt("hello from the session append-only replay contract")
            .await?;

        let body = runtime
            .wait_for_state_rows(
                &[
                    "\"type\":\"connection\"",
                    "\"type\":\"session\"",
                    "\"type\":\"prompt_turn\"",
                    "\"type\":\"chunk\"",
                ],
                DEFAULT_TIMEOUT,
            )
            .await
            .context("INVARIANT (Session): baseline envelope set must land in the stream")?;

        assert!(
            !body.is_empty(),
            "INVARIANT (Session): replay from offset 0 returns a non-empty body"
        );

        let session_idx = body
            .find("\"type\":\"session\"")
            .context("session envelope must appear in replay")?;
        let prompt_turn_idx = body
            .find("\"type\":\"prompt_turn\"")
            .context("prompt_turn envelope must appear in replay")?;
        let chunk_idx = body
            .find("\"type\":\"chunk\"")
            .context("chunk envelope must appear in replay")?;

        assert!(
            session_idx < prompt_turn_idx,
            "INVARIANT (Session): session envelope appears before prompt_turn in append order"
        );
        assert!(
            prompt_turn_idx < chunk_idx,
            "INVARIANT (Session): prompt_turn appears before chunk in append order"
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}

/// Precondition: a control-plane-backed runtime using a shared external
/// durable-streams deployment (not an embedded one) has written events to
/// its durable state stream and has then been shut down cleanly via
/// `POST /v1/runtimes/{key}/stop`.
///
/// Action: after the runtime's process is gone, open a fresh
/// durable-streams reader against the same shared state stream URL and
/// read every event. This replays from `Offset::Beginning` through a
/// completely different process than the one that wrote them.
///
/// Observable evidence: the replay contains typed session records (via
/// the `DecodedStateEntity::Session` decoder path) for the session that
/// was created before shutdown. The stream server itself is still alive
/// because it was spawned by `ControlPlaneHarness`, not by the runtime.
///
/// Invariant proven: **Session durability across runtime death** — the
/// log is not tied to the lifetime of the runtime that wrote it. Any
/// authenticated reader can still consume past events after the runtime
/// is gone, which is the foundational property that makes
/// `resume(sessionId)` cold-start possible.
#[tokio::test]
async fn session_durable_stream_survives_runtime_death() -> Result<()> {
    let control_plane = ControlPlaneHarness::spawn(false).await?;

    let result = async {
        // Provision a runtime through the control plane. The control plane's
        // harness already points runtimes at its shared durable-streams
        // server, so the stream URL will outlive any individual runtime.
        let runtime = control_plane
            .create_runtime("session-durable-after-death")
            .await?;

        // Create a session and drive a prompt so the stream has something
        // to verify against after shutdown.
        let session_id = managed_agent_suite::create_session(&runtime.acp.url).await?;
        prompt_session(
            &runtime.acp.url,
            &session_id,
            "hello before the runtime dies",
        )
        .await?;

        // Wait until the session record is typed-decodable from the stream.
        // This proves the session catalog write landed durably.
        let _record =
            wait_for_session_record(&runtime.state.url, &session_id, DEFAULT_TIMEOUT).await?;

        // Also wait until at least one prompt_turn envelope is visible before
        // shutdown. Without this, the prompt_turn write can still be in flight
        // when the runtime is stopped, and the post-death assertion becomes a
        // flake about timing instead of a durability check.
        let _prompt_turns =
            wait_for_event_count(&runtime.state.url, "prompt_turn", 1, DEFAULT_TIMEOUT).await?;

        // Kill the runtime through the control plane. The runtime process
        // goes away; the stream server does not.
        control_plane.stop_runtime(&runtime.runtime_key).await?;

        // NOW verify the stream still has the session record, read by a
        // completely fresh reader against the shared stream URL.
        let records_after_death = read_session_records(&runtime.state.url).await?;
        assert!(
            records_after_death
                .iter()
                .any(|r| r.session_id == session_id),
            "INVARIANT (Session): shared stream must retain session record after \
             runtime process exit; found {} records total",
            records_after_death.len()
        );

        // Also verify that at least one prompt_turn envelope survived — this
        // is the "progress was recorded before death" half of the contract.
        let prompt_turns_after_death =
            count_events(&runtime.state.url, "prompt_turn").await?;
        assert!(
            prompt_turns_after_death >= 1,
            "INVARIANT (Session): prompt_turn envelopes must survive runtime death, \
             saw {prompt_turns_after_death}"
        );

        Ok(())
    }
    .await;

    control_plane.shutdown().await;
    result
}

/// Precondition: a local runtime has emitted a normal set of events to its
/// durable state stream.
///
/// Action: compute a cursor offset roughly at the mid-point of the current
/// stream length, open a fresh reader at that offset, and collect the resulting
/// body.
///
/// Observable evidence: the mid-offset replay is a strict suffix of the full
/// replay from the beginning — everything before the cursor is absent, and
/// everything at or after the cursor is present in the same order.
///
/// Invariant proven: **Session replay from any offset** — readers can open the
/// log at any point, not just the beginning, and receive the suffix from there
/// forward. This is the contract that makes materializer catch-up semantics
/// possible (start from last-seen offset, drain, then live-tail).
#[tokio::test]
#[ignore = "pending: durable-streams mid-offset replay needs byte-level offset arithmetic; \
            capture-and-replay from a stored offset handle is the more faithful shape"]
async fn session_replay_from_mid_offset_is_suffix_of_full_replay() -> Result<()> {
    pending_contract(
        "session.replay_from_mid_offset",
        "Open a reader at an offset handle produced mid-stream and assert the result is a \
         strict suffix of the full-offset-0 replay. Requires exercising the durable-streams \
         Offset::Handle API rather than just Beginning.",
    )
}

/// Precondition: a fresh durable-streams producer attached to a runtime-scoped
/// stream.
///
/// Action: append the same `StateEnvelope` with the same entity key twice,
/// flushing between the calls. Simulates a producer retry of a write that was
/// acknowledged but whose network round-trip got lost.
///
/// Observable evidence: reading the resulting stream yields the event exactly
/// once, or yields both copies with a monotonic offset that a consumer can
/// deduplicate by entity-key semantics.
///
/// Invariant proven: **Session idempotent append** — Anthropic's "accepts
/// idempotent appends" contract holds under producer retry. This is the
/// contract the external-producer cases (approval service writes, webhook
/// ingest, Flamecast-level writes) rely on.
#[tokio::test]
#[ignore = "pending: need to pin down the durable-streams idempotency contract — is it \
            producer-name+offset dedup, entity-key upsert, or at-least-once with consumer \
            dedup? The test should match the documented guarantee, not assume one."]
async fn session_idempotent_append_under_retry() -> Result<()> {
    pending_contract(
        "session.idempotent_append",
        "Pin the durable-streams idempotency semantics first (producer-name based, \
         entity-key upsert, or consumer-side dedup). Then add two appends of the same \
         envelope with an intentional flush between them and assert the documented \
         behavior. This closes Anthropic's 'accepts idempotent appends' clause.",
    )
}

/// Precondition: a local runtime has generated a normal set of events and the
/// test has both the raw stream body and a materialized SessionIndex over it.
///
/// Action: compare the distinct sessions surfaced by the materializer against
/// the session envelopes visible in the raw stream body.
///
/// Observable evidence: every session id surfaced by the materializer appears
/// in the raw stream, and no extra sessions are invented by the fold.
///
/// Invariant proven: **Session raw-vs-materialized agreement** — the fold over
/// the log is a pure function of the log. Materializers never fabricate,
/// drop, or reorder facts relative to what the raw stream shows.
#[tokio::test]
async fn session_materialized_state_agrees_with_raw_stream() -> Result<()> {
    let runtime = LocalRuntimeHarness::spawn("session-materialized-vs-raw").await?;

    let result = async {
        // Drive a prompt so the stream contains at least one session envelope.
        let _ = runtime
            .prompt("hello from the materialized-vs-raw contract")
            .await?;

        // Wait until the baseline session envelope has landed in the raw stream
        // before we materialize. This ensures both oracles see the same prefix.
        runtime
            .wait_for_state_rows(&["\"type\":\"session\""], DEFAULT_TIMEOUT)
            .await
            .context(
                "INVARIANT (Session): a session envelope must land before materialization",
            )?;

        // Oracle A: the production projector. `materialize_session_index` runs
        // `RuntimeMaterializer` over the raw stream and folds it into a real
        // `SessionIndex`, then aborts the live tail. Whatever it surfaces is
        // exactly what production code would surface for the same stream.
        let index = materialize_session_index(runtime.state_stream_url()).await?;
        let materialized_ids: HashSet<String> = index
            .list()
            .await
            .into_iter()
            .map(|record| record.session_id)
            .collect();

        // Oracle B: the typed raw decoder. `read_session_records` reads every
        // envelope on the stream and decodes the session ones via the same
        // typed `DecodedStateEntity::Session` path the rest of the suite uses.
        // It is NOT the projector — it is the flat list of session envelopes,
        // not a fold over operations.
        let raw_records = read_session_records(runtime.state_stream_url()).await?;
        let raw_ids: HashSet<String> = raw_records
            .iter()
            .map(|record| record.session_id.clone())
            .collect();

        assert!(
            !raw_ids.is_empty(),
            "INVARIANT (Session): the raw stream must contain at least one session \
             envelope after a prompt; otherwise this test proves nothing"
        );
        assert!(
            !materialized_ids.is_empty(),
            "INVARIANT (Session): the materialized SessionIndex must surface at least \
             one session id after a prompt; otherwise this test proves nothing"
        );

        let only_in_materialized: Vec<&String> =
            materialized_ids.difference(&raw_ids).collect();
        let only_in_raw: Vec<&String> = raw_ids.difference(&materialized_ids).collect();

        assert!(
            only_in_materialized.is_empty(),
            "INVARIANT (Session): SessionIndex fabricated session ids absent from the \
             raw stream: {only_in_materialized:?}"
        );
        assert!(
            only_in_raw.is_empty(),
            "INVARIANT (Session): SessionIndex dropped session ids visible in the raw \
             stream: {only_in_raw:?}"
        );
        assert_eq!(
            materialized_ids, raw_ids,
            "INVARIANT (Session): materialized SessionIndex must be a pure function of \
             the raw stream — id sets must match exactly"
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}
