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
use managed_agent_suite::{DEFAULT_TIMEOUT, LocalRuntimeHarness, pending_contract};

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

/// Precondition: a control-plane-backed runtime (using a shared external
/// durable-streams deployment, not an embedded one) has written events to its
/// durable state stream and has then been shut down cleanly.
///
/// Action: after the runtime process is gone, open a fresh durable-streams
/// reader against the same shared state stream URL and replay from
/// `Offset::Beginning`.
///
/// Observable evidence: the replay still contains the envelopes emitted by the
/// now-dead runtime, including the `prompt_turn` and `chunk` events from the
/// prompt that was sent before shutdown.
///
/// Invariant proven: **Session durability across runtime death** — the log is
/// not tied to the lifetime of the runtime that wrote it. Any authenticated
/// reader can still consume past events after the runtime is gone, which is the
/// foundational property that makes `resume(sessionId)` cold-start possible.
///
/// Known blocker: `LocalRuntimeHarness::spawn` currently brings up an EMBEDDED
/// durable-streams server per runtime, which dies with the runtime — running
/// this test against that harness would produce a false negative (the test
/// would fail not because the Session contract is broken, but because the
/// embedded stream server went away). The faithful test needs a
/// `ControlPlaneHarness` variant that stands up a shared stream outside the
/// runtime's lifetime and lets the test prompt the runtime via the control
/// plane's advertised ACP endpoint.
#[tokio::test]
#[ignore = "pending: needs ControlPlaneHarness + prompt helper to separate stream \
            lifecycle from runtime lifecycle; LocalRuntimeHarness's embedded stream \
            dies with the runtime and produces a false negative"]
async fn session_durable_stream_survives_runtime_death() -> Result<()> {
    pending_contract(
        "session.durable_stream_survives_runtime_death",
        "Extend ControlPlaneHarness with an ACP prompt helper (or add a \
         LocalRuntimeHarness::spawn_with_shared_stream variant), then: prompt the runtime, \
         shut it down via POST /v1/runtimes/{key}/stop, open a fresh DsClient against the \
         shared stream URL, and assert the session/prompt_turn envelopes are still present. \
         The current LocalRuntimeHarness embeds the durable-streams server in the runtime \
         process so this test cannot run faithfully against it.",
    )
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
#[ignore = "pending: promote once the SessionIndex materializer exposes a deterministic \
            list of session ids and the raw stream exposes a stable session envelope shape \
            that a test can parse without reimplementing the projector"]
async fn session_materialized_state_agrees_with_raw_stream() -> Result<()> {
    pending_contract(
        "session.materialized_vs_raw_agreement",
        "Read the raw stream body, parse session envelopes, cross-check each one against \
         a materialized SessionIndex. The test must NOT reimplement the projector — if it \
         does, it is circular. Promote once the SessionIndex exposes a list+get surface a \
         test can consume against parsed raw envelopes.",
    )
}
