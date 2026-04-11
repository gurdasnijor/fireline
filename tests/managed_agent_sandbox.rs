//! # Sandbox Primitive Contract Tests
//!
//! Validates the **Sandbox** managed-agent primitive against the acceptance bars
//! in `docs/explorations/managed-agents-mapping.md` §4 "Sandbox" and the
//! Anthropic interface:
//!
//! ```text
//! provision({resources}) → execute(name, input) → String
//! ```
//!
//! *"Any executor that can be configured once and called many times as a tool —
//! a local process, a remote container, etc."*
//!
//! Sandbox is split across `client.host` (provision lifecycle) and `client.acp`
//! (execute channel) in Fireline's TS surface. The Rust substrate side is the
//! `RuntimeProvider` trait plus `LocalProvider`, `ChildProcessRuntimeLauncher`,
//! and `DockerProvider`. This file tests both the provision contract (runtime
//! reachable, reusable across many prompts) and the stop+recreate contract
//! (the same spec can be re-provisioned and session/load'ed).
//!
//! Every test follows the explicit oracle shape: precondition, action,
//! observable evidence, invariant proven. Tests that run against current code
//! pass today. Tests that need implementation work are marked `#[ignore]` with
//! a `pending_contract` marker.
//!
//! **Ownership boundary:** this file covers Rust-substrate provision and
//! lifecycle invariants. Cross-provider behavioral equivalence (slice 13c
//! mixed-topology proof) has its own end-to-end test at
//! `tests/control_plane_docker.rs`; per-provider integration specifics belong
//! in provider-local tests, not here.

#[path = "support/managed_agent_suite.rs"]
mod managed_agent_suite;

use std::time::Duration;

use anyhow::{Context, Result};
use managed_agent_suite::{LocalRuntimeHarness, pending_contract, wait_for_event_count};

/// Precondition: no runtime exists.
///
/// Action: `LocalRuntimeHarness::spawn` — which under the hood invokes
/// `bootstrap::start(BootstrapConfig)` and runs the full provision path
/// (component chain build, ACP listener bind, state stream producer setup).
///
/// Observable evidence: the returned harness exposes non-empty ACP and state
/// stream URLs, and an immediate ACP prompt against the ACP URL succeeds,
/// confirming the listener is actually accepting traffic.
///
/// Invariant proven: **Sandbox provision contract** — `provision()` returns a
/// runtime that is reachable at its advertised endpoints. The act of returning
/// from provision is a promise that the runtime is ready to receive execute
/// calls, not merely that the process has started.
#[tokio::test]
async fn sandbox_provision_returns_reachable_runtime() -> Result<()> {
    let runtime = LocalRuntimeHarness::spawn("sandbox-provision-reachable").await?;

    let result = async {
        assert!(
            !runtime.acp_url().is_empty(),
            "INVARIANT (Sandbox): provisioned runtime advertises a non-empty ACP endpoint"
        );
        assert!(
            !runtime.state_stream_url().is_empty(),
            "INVARIANT (Sandbox): provisioned runtime advertises a non-empty state stream"
        );

        let response = runtime
            .prompt("sandbox provision reachability probe")
            .await
            .context(
                "INVARIANT (Sandbox): ACP endpoint must accept traffic immediately \
                 after provision returns",
            )?;

        assert!(
            !response.is_empty(),
            "INVARIANT (Sandbox): first execute call after provision returns a response"
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}

/// Precondition: a runtime has been provisioned and served at least one
/// successful ACP prompt.
///
/// Action: send several additional prompts against the same runtime
/// sequentially without re-provisioning. Each prompt is a discrete Sandbox
/// `execute(name, input)` call under the Anthropic framing; the Fireline
/// mapping is that each ACP `session/prompt` request is one execution.
///
/// Observable evidence: every prompt returns a non-empty response without
/// requiring the runtime to be torn down or re-provisioned, and the runtime's
/// durable state stream accumulates envelopes for each turn.
///
/// Invariant proven: **Sandbox configured-once-called-many-times** — this is
/// Anthropic's literal wording for the Sandbox contract. A single
/// `provision()` backs arbitrarily many `execute()` calls for the runtime's
/// lifetime.
#[tokio::test]
async fn sandbox_provisioned_runtime_serves_multiple_execute_calls() -> Result<()> {
    let runtime = LocalRuntimeHarness::spawn("sandbox-multi-execute").await?;

    let result = async {
        for (iteration, prompt) in [
            "first execute against this sandbox",
            "second execute — still the same runtime",
            "third execute — no re-provision",
        ]
        .iter()
        .enumerate()
        {
            let response = runtime.prompt(prompt).await.with_context(|| {
                format!(
                    "INVARIANT (Sandbox): execute #{} must succeed against a \
                     configured-once runtime",
                    iteration + 1
                )
            })?;
            assert!(
                !response.is_empty(),
                "INVARIANT (Sandbox): execute #{} returned a non-empty response",
                iteration + 1
            );
        }

        // Each prompt should land as a distinct prompt_turn envelope on the
        // durable stream. Use count-aware polling to avoid the race where
        // the substring helper returns after only the first prompt_turn is
        // visible.
        let prompt_turns = wait_for_event_count(
            runtime.state_stream_url(),
            "prompt_turn",
            3,
            Duration::from_secs(10),
        )
        .await
        .context(
            "INVARIANT (Sandbox ∘ Session): each execute yields a distinct prompt_turn \
             envelope in the durable log",
        )?;
        assert!(
            prompt_turns.len() >= 3,
            "INVARIANT (Sandbox ∘ Session): wait_for_event_count returned {} prompt_turn \
             envelopes, expected at least 3",
            prompt_turns.len()
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}

/// Precondition: a `ControlPlaneHarness`-backed runtime was provisioned against
/// a shared external durable stream, served at least one prompt, and was
/// stopped via `POST /v1/runtimes/{key}/stop`.
///
/// Action: provision a fresh runtime against the **same** stored spec
/// (identical `runtime_key` and `agent_command`), then issue an ACP
/// `session/load` for the session id that was created in the first runtime.
///
/// Observable evidence: the `session/load` call succeeds and the resumed
/// runtime can serve follow-up prompts against the same session state that
/// existed at shutdown time.
///
/// Invariant proven: **Sandbox stop + recreate equivalence** — the provision
/// contract is stable enough that re-provisioning against the stored spec
/// produces a runtime that is behaviorally equivalent to the original for the
/// purposes of ACP `session/load`. This is the contract the `resume(sessionId)`
/// helper in `src/orchestration.rs` relies on.
#[tokio::test]
#[ignore = "pending: requires a ControlPlaneHarness prompt helper and the resume helper \
            wiring currently being debugged in managed_agent_primitives_suite.rs; will \
            promote to real once the orchestration contract test passes"]
async fn sandbox_stop_and_recreate_preserves_session_load() -> Result<()> {
    pending_contract(
        "sandbox.stop_and_recreate_session_load",
        "Needs: (1) ControlPlaneHarness ACP prompt helper (not yet in the shared harness), \
         (2) resume(sessionId) helper debugged and passing in managed_agent_primitives_suite. \
         The test should provision via control plane, prompt, stop via control plane, \
         call src/orchestration.rs resume, and verify the resumed runtime can load_session \
         and serve a follow-up prompt. Promote once codex DAR's orchestration debug lands.",
    )
}

/// Precondition: a `LocalProvider` and a `DockerProvider` are both available
/// and can be pointed at the same shared durable stream.
///
/// Action: provision one runtime via each provider using equivalent launch
/// specs, then compare their observable behavior — are advertised endpoints
/// the same shape, do they respond to ACP `session/prompt` the same way, do
/// they write the same envelope types to the stream?
///
/// Observable evidence: both runtimes expose the same surface shape; both
/// accept prompts and append events via the shared conductor chain; both
/// survive the same `session/load` flow.
///
/// Invariant proven: **Sandbox cross-provider behavioral equivalence** —
/// swapping `LocalProvider` for `DockerProvider` (or later E2B, Daytona) does
/// not change the observable substrate contract. This is the promise that the
/// `RuntimeProvider` trait is actually an abstraction and not a leaky one.
#[tokio::test]
#[ignore = "already covered end-to-end by tests/control_plane_docker.rs in its slice 13c \
            mixed-topology proof; this marker exists to make the primitive coverage \
            explicit and to document that the per-provider contract lives elsewhere"]
async fn sandbox_cross_provider_behavioral_equivalence() -> Result<()> {
    pending_contract(
        "sandbox.cross_provider_equivalence",
        "Covered by tests/control_plane_docker.rs (slice 13c mixed-topology proof). \
         This pending marker exists so the primitive suite acknowledges the contract \
         without duplicating the heavy Docker-spawning test here. If cross-provider \
         coverage regresses, unignore this and either call into the existing test or \
         port a focused subset.",
    )
}
