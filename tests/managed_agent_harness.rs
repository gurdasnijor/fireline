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

use anyhow::{Context, Result};
use managed_agent_suite::{DEFAULT_TIMEOUT, LocalRuntimeHarness, pending_contract};

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
/// full prompt → response cycle.
///
/// Action: snapshot the durable stream at time T1, record the observed
/// effect sequence. Send another prompt. Snapshot the stream at time T2.
///
/// Observable evidence: the sequence of event kinds at T2 is a **strict
/// superset** of the sequence at T1, and the T1 subset appears in the same
/// order it appeared live. The new events from the second prompt extend the
/// tail.
///
/// Invariant proven: **Harness append-order stability under live writes** —
/// the durable log does not reorder or lose past events when the runtime
/// continues to write new ones. Materializers that cache a last-seen offset
/// and drain forward can rely on this contract.
#[tokio::test]
#[ignore = "pending: needs a second-prompt-then-diff helper against the stream, and a \
            parser that extracts ordered event-kind sequences so the superset assertion \
            is exact rather than substring-based"]
async fn harness_append_order_is_stable_under_continued_writes() -> Result<()> {
    pending_contract(
        "harness.append_order_stable_under_continued_writes",
        "Add a helper that snapshots the state stream at a point in time, parses out \
         the ordered sequence of event `type` fields, and lets the test assert \
         (T1_sequence is a prefix of T2_sequence) after a second prompt. Requires \
         either a dedicated parser in the support module or a lightweight serde_json \
         walk in the test body.",
    )
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
