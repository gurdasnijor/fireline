//! # Harness Primitive Contract Tests
//!
//! Validates the **Harness** managed-agent primitive against the acceptance bars
//! in `docs/explorations/managed-agents-mapping.md` §3 "Harness" and the
//! Anthropic interface:
//!
//! ```text
//! yield Effect<T> → EffectResult<T>
//! ```
//!
//! *"Any loop that yields effects and appends progress to the Session."*
//!
//! Fireline implements the Harness primitive at a different layer than
//! Anthropic's framing: the harness is the agent process, and the conductor
//! proxy chain sits between the harness and its effects. Every effect the
//! agent yields (ACP requests) flows through the chain, where components can
//! observe, transform, filter, suspend, substitute, or fan out — the seven-
//! combinator algebra described in the mapping doc §"Fireline as combinators
//! over the primitives".
//!
//! This file validates Harness contracts at the **substrate level** — that
//! effects actually land in the durable log, that the proxy chain composes,
//! that suspend/resume survives runtime death via event sourcing. The full
//! combinator algebra (observe/mapEffect/appendToSession/filter/substitute/
//! suspend/fanout as distinct composable shapes) is a Fireline-internal
//! design, not an Anthropic primitive invariant, so it gets a lighter check.
//!
//! **Ownership boundary:** component-specific behavior (ApprovalGate policy,
//! Budget token accounting, ContextInjection content sources) belongs in
//! component unit tests, not here. This file checks the Harness **seam**
//! contracts.

#[path = "support/managed_agent_suite.rs"]
mod managed_agent_suite;

use std::time::Duration;

use anyhow::{Context, Result};
use managed_agent_suite::{
    DEFAULT_TIMEOUT, LocalRuntimeHarness, count_events, pending_contract, read_all_events,
    wait_for_event_count,
};

/// Precondition: a local runtime has been provisioned with the default
/// topology (whatever the baseline harness sets up).
///
/// Action: prompt the runtime once via ACP.
///
/// Observable evidence: the runtime's durable state stream contains
/// `session`, `prompt_turn`, and `chunk` envelopes. This is the minimal
/// effect set for a non-trivial prompt — anything less would mean the
/// conductor proxy chain failed to append progress for at least one
/// observable event.
///
/// Invariant proven: **Harness ∘ Session — every effect is appended to the
/// durable log.** The conductor's `DurableStreamTracer` component captures
/// every ACP-level effect as it flows through the proxy chain, so nothing the
/// harness yields bypasses the durable record. This is the Anthropic "appends
/// progress to the Session" clause.
#[tokio::test]
async fn harness_every_effect_is_appended_to_session_log() -> Result<()> {
    let runtime = LocalRuntimeHarness::spawn("harness-effects-logged").await?;

    let result = async {
        let _ = runtime
            .prompt("harness durable-log effect contract")
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
            .context(
                "INVARIANT (Harness ∘ Session): every effect the harness yields must \
                 land in the durable state stream via the conductor proxy chain",
            )?;

        let connection_count = body.matches("\"type\":\"connection\"").count();
        let session_count = body.matches("\"type\":\"session\"").count();
        let prompt_turn_count = body.matches("\"type\":\"prompt_turn\"").count();
        let chunk_count = body.matches("\"type\":\"chunk\"").count();

        assert!(
            connection_count >= 1,
            "connection envelope count = {connection_count}"
        );
        assert!(session_count >= 1, "session envelope count = {session_count}");
        assert!(
            prompt_turn_count >= 1,
            "prompt_turn envelope count = {prompt_turn_count}"
        );
        assert!(
            chunk_count >= 1,
            "chunk envelope count = {chunk_count} (at least one message chunk expected)"
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}

/// Precondition: a runtime has been provisioned and has run at least one
/// full prompt → response cycle, producing a set of events in its
/// durable state stream.
///
/// Action: (1) snapshot the durable stream at time T1 via
/// `read_all_events` and extract the ordered sequence of envelope type
/// strings; (2) send another prompt; (3) wait for the event count to
/// increase via `wait_for_event_count` on prompt_turn; (4) snapshot again
/// at time T2 and extract the new ordered sequence.
///
/// Observable evidence: the T1 sequence is an exact prefix of the T2
/// sequence. Every element at every index matches. This proves no
/// reordering and no loss of past events when the runtime continues to
/// write new ones. Done with parsed event types via `StateEnvelope`, not
/// substring matching.
///
/// Invariant proven: **Harness append-order stability under live writes**
/// — the durable log does not reorder or lose past events when the
/// runtime continues to write new ones. Materializers that cache a
/// last-seen offset and drain forward can rely on this contract.
#[tokio::test]
async fn harness_append_order_is_stable_under_continued_writes() -> Result<()> {
    let runtime = LocalRuntimeHarness::spawn("harness-append-order-stable").await?;

    let result = async {
        // First prompt — seed the stream.
        let _ = runtime.prompt("first prompt to seed the log").await?;
        let _ = wait_for_event_count(
            runtime.state_stream_url(),
            "prompt_turn",
            1,
            DEFAULT_TIMEOUT,
        )
        .await
        .context("first prompt's prompt_turn must land in the log")?;

        // T1 snapshot: extract the ordered type sequence.
        let t1_events = read_all_events(runtime.state_stream_url()).await?;
        let t1_sequence: Vec<String> = t1_events
            .iter()
            .filter_map(|env| env.envelope_type().map(str::to_string))
            .collect();
        assert!(
            !t1_sequence.is_empty(),
            "T1 sequence must contain at least the initial envelopes"
        );

        // Second prompt — continue writing to the live stream.
        let _ = runtime.prompt("second prompt after T1 snapshot").await?;

        // Wait until the prompt_turn count is strictly greater than the T1
        // count — confirms the second prompt landed.
        let t1_prompt_turn_count = t1_sequence
            .iter()
            .filter(|kind| *kind == "prompt_turn")
            .count();
        let _ = wait_for_event_count(
            runtime.state_stream_url(),
            "prompt_turn",
            t1_prompt_turn_count + 1,
            Duration::from_secs(10),
        )
        .await
        .context("second prompt's prompt_turn must land in the log")?;

        // T2 snapshot.
        let t2_events = read_all_events(runtime.state_stream_url()).await?;
        let t2_sequence: Vec<String> = t2_events
            .iter()
            .filter_map(|env| env.envelope_type().map(str::to_string))
            .collect();

        // The critical assertion: T1 is a strict prefix of T2. Every
        // element at every index matches, and T2 is strictly longer.
        assert!(
            t2_sequence.len() > t1_sequence.len(),
            "INVARIANT (Harness): T2 sequence must be strictly longer than T1 after a \
             second prompt; T1 len = {}, T2 len = {}",
            t1_sequence.len(),
            t2_sequence.len()
        );

        for (index, (t1_kind, t2_kind)) in t1_sequence.iter().zip(t2_sequence.iter()).enumerate()
        {
            assert_eq!(
                t1_kind, t2_kind,
                "INVARIANT (Harness): T1 must be an exact prefix of T2 (no reordering, \
                 no loss). Mismatch at index {index}: T1 = '{t1_kind}', T2 = '{t2_kind}'"
            );
        }

        // Sanity-check via the count_events helper.
        let final_prompt_turn_count = count_events(runtime.state_stream_url(), "prompt_turn")
            .await?;
        assert!(
            final_prompt_turn_count >= t1_prompt_turn_count + 1,
            "INVARIANT (Harness): final prompt_turn count must be strictly greater than \
             the T1 count"
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}

/// Precondition: a runtime has been provisioned with an `ApprovalGateComponent`
/// in its topology, and the agent has yielded a tool call that hits the
/// approval gate and suspends.
///
/// Action: simulate the runtime dying mid-suspension (stop the runtime via
/// control plane), then re-provision a fresh runtime against the same stored
/// spec and issue `session/load` for the suspended session id.
///
/// Observable evidence: the freshly started runtime observes the pending
/// approval event on the Session log during `session/load`, rebuilds the
/// component's pending state from the log, and releases the pause when the
/// approval is resolved (either by a prior Allow event on the log or by a
/// new external append).
///
/// Invariant proven: **Harness durable suspend/resume** — the conductor
/// suspend combinator is event-sourced. A component can pause mid-effect,
/// the pause survives runtime death, and a new runtime can rebuild the
/// paused state from the log and continue. This is the Harness half of the
/// Orchestration composition reduction.
#[tokio::test]
#[ignore = "pending: slice 16 ApprovalGateComponent rebuild-from-log behavior (currently \
            uncommitted in crates/fireline-components/src/approval.rs) and the scripted \
            testy harness needed to reliably trigger a tool call that hits the approval gate"]
async fn harness_durable_suspend_resume_round_trip() -> Result<()> {
    pending_contract(
        "harness.durable_suspend_resume",
        "Blocks on (1) codex DAR committing the ApprovalGateComponent rebuild-from-log \
         work in crates/fireline-components/src/approval.rs, and (2) a scripted testy \
         agent that deterministically emits a tool call the approval gate will suspend. \
         Without (2), the test has to rely on the real agent's nondeterministic behavior, \
         which isn't acceptable for a golden contract test. Promote once both land.",
    )
}

/// Precondition: a topology is constructed with a representative instance of
/// each of the seven combinator kinds (observe, mapEffect, appendToSession,
/// filter, substitute, suspend, fanout).
///
/// Action: inspect the resulting `TopologySpec` and assert that each
/// combinator kind is actually represented as a distinct component with the
/// expected internal shape.
///
/// Observable evidence: the component list contains exactly seven distinct
/// combinator kinds, each identified by its kind tag or config shape.
///
/// Invariant proven: **Fireline-internal combinator algebra coverage** —
/// this is a Fireline-specific design invariant, not an Anthropic primitive
/// contract. It's included here to prevent the combinator algebra from
/// silently drifting away from the seven documented shapes in
/// `managed-agents-mapping.md` §"Fireline as combinators over the primitives".
/// **Not a managed-agent acceptance bar.**
#[tokio::test]
#[ignore = "pending: requires TopologySpec to expose a kind-tagged view of its components \
            rather than an opaque Vec, or a helper that walks the components and counts \
            distinct combinator kinds — currently topology_component_count() only returns \
            a Vec::len(), which cannot distinguish seven identical components from seven \
            distinct ones"]
async fn harness_topology_represents_all_seven_combinators() -> Result<()> {
    pending_contract(
        "harness.combinator_algebra_coverage",
        "The existing topology_component_count() check in the E2E spec only validates \
         Vec::len(), which is structurally correct but doesn't prove the seven distinct \
         combinator kinds are each present. Expose a kind-tagged view (e.g. \
         HashSet<CombinatorKind>) on TopologySpec or in a test helper, then assert the \
         set contains all seven. NOTE: this is a Fireline-internal invariant, not an \
         Anthropic primitive acceptance bar — the mapping doc's 'seven combinators' is a \
         design framing, not a contract the Anthropic post defines.",
    )
}
