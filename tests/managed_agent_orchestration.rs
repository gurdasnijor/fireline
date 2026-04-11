//! # Orchestration Primitive Contract Tests
//!
//! Validates the **Orchestration** managed-agent primitive against the
//! acceptance bars in `docs/explorations/managed-agents-mapping.md` §2
//! "Orchestration" and the Anthropic interface:
//!
//! ```text
//! wake(session_id) → void
//! ```
//!
//! *"Any scheduler that can call a function with an ID and retry on failure —
//! a cron job, a queue consumer, a while-loop, etc."*
//!
//! The critical reduction in the mapping doc §2 is that Orchestration is
//! **satisfied by composition of existing primitives** — not by a new scheduler
//! service. The composition is a ten-line helper:
//!
//! ```text
//! resume(sessionId) =
//!   sessionStore.get(sessionId)           — Session read surface
//! → provision(storedRuntimeSpec)          — Sandbox provision
//! → connectAcp(runtime.acp)               — ACP transport
//! → loadSession(sessionId)                — session/load rebuild
//! ```
//!
//! Any process that subscribes to the Session stream and calls `resume` in
//! response to events is a scheduler. Retries fall out of subscription offset
//! tracking. No new primitive needed.
//!
//! This file validates the Orchestration composition contract. The main test
//! (`managed_agent_orchestration_acceptance_contract`) currently lives in
//! `tests/managed_agent_primitives_suite.rs` and is being debugged by codex
//! DAR — see the cross-reference below. Additional contract tests live here.
//!
//! **Ownership boundary:** the in-process Rust test side owns the
//! composition helper wiring and the cold-start cycle proof. The
//! TypeScript client surface (`resume(sessionId)` in `@fireline/client`)
//! and subscriber-coordination ergonomics live in `packages/client` tests.

#[path = "support/managed_agent_suite.rs"]
mod managed_agent_suite;

use anyhow::Result;
use managed_agent_suite::pending_contract;

/// Cross-reference: the end-to-end cycle proof for Orchestration composition
/// lives at
/// `tests/managed_agent_primitives_suite.rs::managed_agent_orchestration_acceptance_contract`.
/// That test spawns a `ControlPlaneHarness`, creates a runtime via
/// `create_runtime_with_agent`, asserts `reconstruct_runtime_spec_from_log`
/// sees the persisted spec, creates a session, prompts, stops the runtime,
/// calls `src/orchestration.rs::resume`, and asserts the resumed runtime is
/// Ready with a new runtime_id under the same runtime_key.
///
/// That's the load-bearing Orchestration cycle test. It's being actively
/// debugged by codex DAR — their fix for the register/ready race is in
/// flight on `crates/fireline-conductor/src/runtime/mod.rs`. Once it lands
/// and passes, the primary Orchestration acceptance gate is green.
///
/// This file adds ADDITIONAL contract tests that the primitive suite's
/// single narrative-shaped test doesn't cover. Nothing here duplicates that
/// test's work.
///
/// Precondition: a session has been established in a live runtime; the
/// runtime has NOT been stopped.
///
/// Action: call `resume(sessionId)` against the live runtime.
///
/// Observable evidence: the call returns the same runtime descriptor
/// (same runtime_key, same runtime_id, status Ready) without spinning up a
/// new runtime — the resume helper's short-circuit path is taken.
///
/// Invariant proven: **Orchestration idempotent on live runtimes** — calling
/// `resume` against a runtime that's already serving traffic is a no-op, not
/// a cold-start. This is the retry-safety property: a subscriber that sees
/// the same "needs to advance" event twice and calls `resume` twice does not
/// destabilize the runtime.
#[tokio::test]
#[ignore = "pending: primary orchestration cycle test in managed_agent_primitives_suite \
            must pass first; this idempotency-on-live variant adds value only after the \
            base contract is green, and it needs a ControlPlaneHarness ACP prompt helper"]
async fn orchestration_resume_on_live_runtime_is_noop() -> Result<()> {
    pending_contract(
        "orchestration.resume_idempotent_on_live",
        "Requires (1) the primary managed_agent_orchestration_acceptance_contract in \
         managed_agent_primitives_suite.rs to pass first — blocks on codex DAR's \
         register/ready race fix, and (2) a ControlPlaneHarness ACP prompt helper so \
         the test can create a session without hand-rolling ACP plumbing. Promote after \
         both land.",
    )
}

/// Precondition: a session has been established, the runtime has been
/// stopped via `POST /v1/runtimes/{key}/stop`, and the `runtimeSpec` is
/// durably persisted in the Session log (verified separately by the
/// primary orchestration contract).
///
/// Action: call `resume(sessionId)` TWO TIMES CONCURRENTLY via
/// `tokio::join!`. Both calls hit the cold-start path simultaneously.
///
/// Observable evidence: both calls return runtime descriptors with the
/// **same** `runtime_key`, and only **one** new runtime process is
/// instantiated by the control plane (verified by counting `create` events
/// in the stream or by checking the control plane's runtime list
/// increment).
///
/// Invariant proven: **Orchestration concurrent-resume idempotency** —
/// this is the property that was falsely claimed in the e2e spec's
/// sequential "second resume" test. Real concurrency via `tokio::join!`
/// proves the mapping doc §2 claim that "two subscribers seeing the same
/// wake event and both calling `resume` produce exactly one effective
/// resumption."
///
/// This is one of the specific gaps the code reviewer identified in the
/// e2e spec — the existing check was sequential, not concurrent.
#[tokio::test]
#[ignore = "pending: primary orchestration cycle test must pass first (codex DAR's fix), \
            then add a control-plane runtime-count oracle so we can verify only ONE new \
            runtime process was created when two resume calls raced"]
async fn orchestration_concurrent_resume_creates_single_runtime() -> Result<()> {
    pending_contract(
        "orchestration.concurrent_resume_idempotency",
        "Use tokio::join! to fire two resume(sessionId) futures in parallel. Both must \
         return the same runtime_key. Additionally: query ControlPlaneHarness::list_runtimes \
         before and after the concurrent calls and assert the count incremented by exactly \
         one, not two. This is the missing gap the e2e spec reviewer called out — the \
         existing 'concurrent resume' check is sequential and proves nothing about the \
         race.",
    )
}

/// Precondition: a runtime has a pending approval (wait) record on its
/// Session stream, and the runtime has been stopped.
///
/// Action: an external process (simulating an approval service) appends an
/// `approval_resolved` event to the durable stream using a stream-write token
/// issued by the control plane. This is the "external producer" path — no
/// live runtime is involved in the write. A second process subscribes to
/// the stream, observes the `approval_resolved` event, and calls
/// `resume(sessionId)` in response.
///
/// Observable evidence: the resumed runtime's `ApprovalGateComponent`
/// rebuilds its pending state from the log, sees the `approval_resolved`
/// event, and releases the pause. The agent continues from exactly where it
/// was suspended.
///
/// Invariant proven: **Orchestration subscriber-coordination cycle** — the
/// full end-to-end story from the mapping doc §2 walkthrough: external
/// append + subscriber loop + resume + event-sourced pause release. This is
/// the Orchestration composition working at its highest leverage point.
#[tokio::test]
#[ignore = "pending: slice 16 ApprovalGateComponent rebuild-from-log (uncommitted in \
            crates/fireline-components/src/approval.rs) + scripted testy for deterministic \
            tool-call-that-triggers-approval-gate + full cycle wiring through \
            ControlPlaneHarness"]
async fn orchestration_subscriber_loop_drives_pause_release_cycle() -> Result<()> {
    pending_contract(
        "orchestration.subscriber_loop_pause_release",
        "The full §2 walkthrough as a single contract. Blocks on (1) slice 16 approval \
         gate rebuild-from-log committing, (2) scripted testy to deterministically hit \
         the approval gate, (3) ControlPlaneHarness prompt + stream write helpers. This \
         is the capstone Orchestration test — once it passes, the composition reduction \
         is fully validated.",
    )
}
