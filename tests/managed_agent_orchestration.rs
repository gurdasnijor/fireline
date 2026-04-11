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

use anyhow::{Context, Result};
use fireline_conductor::topology::{TopologyComponentSpec, TopologySpec};
use managed_agent_suite::{
    DEFAULT_TIMEOUT, LocalRuntimeHarness, ManagedAgentHarnessSpec, append_approval_resolved,
    count_events, create_session, pending_contract, prompt_session, wait_for_permission_request,
};

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

/// Precondition: a runtime is provisioned with an `approval_gate` topology
/// component whose policy suspends any prompt containing a known needle,
/// and a prompt containing that needle has been fired.
///
/// Action: a separate task simulating an "approval service" tails the
/// durable state stream, waits for the `permission_request` envelope
/// emitted by the gate, extracts the `requestId`, and appends an
/// `approval_resolved` event with `allow: true`. The original prompt
/// task unblocks and the agent completes the turn.
///
/// Observable evidence: the blocked prompt returns successfully, and the
/// stream records the full cycle — `permission_request` then
/// `approval_resolved` — in order.
///
/// Invariant proven: **Orchestration subscriber-coordination cycle** —
/// an external producer, a subscriber that observes the pending event,
/// and an event-sourced pause release together drive a prompt to
/// completion. This is the "two processes on the same stream, no shared
/// memory, no RPC" shape from the mapping doc §2. It does not yet cover
/// the "runtime dies mid-pause" branch; that remains the
/// `harness_durable_suspend_resume_round_trip` case.
#[tokio::test]
async fn orchestration_subscriber_loop_drives_pause_release_cycle() -> Result<()> {
    let topology = TopologySpec {
        components: vec![TopologyComponentSpec {
            name: "approval_gate".to_string(),
            config: Some(serde_json::json!({
                "timeoutMs": 15000,
                "policies": [
                    {
                        "match": { "kind": "promptContains", "needle": "pause_here" },
                        "action": "requireApproval",
                        "reason": "test policy: subscriber loop"
                    }
                ]
            })),
        }],
    };
    let spec = ManagedAgentHarnessSpec::new("orchestration-subscriber-loop").with_topology(topology);
    let runtime = LocalRuntimeHarness::spawn_with(spec).await?;

    let result = async {
        let session_id = create_session(runtime.acp_url()).await?;
        let acp_url = runtime.acp_url().to_string();
        let state_url = runtime.state_stream_url().to_string();

        // Prompt task — blocks in the gate until an approval_resolved
        // event lands on the stream.
        let prompt_session_id = session_id.clone();
        let prompt_acp_url = acp_url.clone();
        let prompt_task = tokio::spawn(async move {
            prompt_session(
                &prompt_acp_url,
                &prompt_session_id,
                "please pause_here for orchestration",
            )
            .await
        });

        // Subscriber/approval-service task — tails the stream, observes
        // the pending permission_request, and writes an
        // approval_resolved response. This is the "external producer +
        // subscriber loop" path from the mapping doc §2 walkthrough.
        let subscriber_session_id = session_id.clone();
        let subscriber_state_url = state_url.clone();
        let subscriber = tokio::spawn(async move {
            let request_id = wait_for_permission_request(
                &subscriber_state_url,
                &subscriber_session_id,
                DEFAULT_TIMEOUT,
            )
            .await?;
            append_approval_resolved(
                &subscriber_state_url,
                &subscriber_session_id,
                &request_id,
                true,
            )
            .await?;
            anyhow::Ok(request_id)
        });

        subscriber
            .await
            .context("subscriber task panicked")?
            .context(
                "INVARIANT (Orchestration): subscriber must observe the permission_request \
                 and append an approval_resolved response",
            )?;

        let prompt_outcome =
            tokio::time::timeout(std::time::Duration::from_secs(15), prompt_task)
                .await
                .context("prompt task did not complete within the post-approval window")?;
        prompt_outcome
            .context("prompt task panicked")?
            .context(
                "INVARIANT (Orchestration): blocked prompt must succeed once the subscriber \
                 loop resolves the approval",
            )?;

        let permission_envelopes = count_events(&state_url, "permission").await?;
        assert!(
            permission_envelopes >= 2,
            "INVARIANT (Orchestration): stream must record both permission_request and \
             approval_resolved envelopes, saw {permission_envelopes}"
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}
