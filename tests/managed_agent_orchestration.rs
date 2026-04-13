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
//! → ACP SDK connect(runtime.acp)          — ACP transport
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
use fireline_harness::{
    ActiveSubscriber, AlwaysOnDeploymentSubscriber, DeploymentWakeRequested, HandlerOutcome,
    TopologyComponentSpec, TopologySpec,
};
use managed_agent_suite::{
    ControlPlaneHarness, DEFAULT_TIMEOUT, LocalRuntimeHarness, ManagedAgentHarnessSpec,
    append_approval_resolved, count_events, create_session, load_session, prompt_session,
    wait_for_permission_request,
};
use sacp::schema::SessionId;

fn expect_completed<T>(outcome: HandlerOutcome<T>, context: &str) -> Result<T> {
    match outcome {
        HandlerOutcome::Completed(completion) => Ok(completion),
        HandlerOutcome::RetryTransient(error) | HandlerOutcome::Failed(error) => {
            Err(error.context(context.to_string()))
        }
    }
}

async fn wait_for_runtime_descriptor_from_control_plane(
    http: &reqwest::Client,
    base_url: &str,
    host_key: &str,
    runtime_id: &str,
) -> Result<fireline_session::HostDescriptor> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let descriptor = http
            .get(format!(
                "{}/v1/runtimes/{}",
                base_url.trim_end_matches('/'),
                host_key
            ))
            .send()
            .await
            .context("fetch runtime descriptor for deployment wake assertion")?
            .error_for_status()
            .context("runtime descriptor endpoint rejected deployment wake assertion fetch")?
            .json::<fireline_session::HostDescriptor>()
            .await
            .context("decode runtime descriptor for deployment wake assertion")?;
        if descriptor.host_id == runtime_id {
            return Ok(descriptor);
        }

        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "timed out waiting for runtime '{runtime_id}' under host_key '{host_key}' from the control-plane runtime descriptor"
            );
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Cross-reference: the end-to-end cycle proof for Orchestration composition
/// lives at
/// `tests/managed_agent_primitives_suite.rs::managed_agent_orchestration_acceptance_contract`.
/// That test spawns a `ControlPlaneHarness`, creates a runtime via
/// `create_runtime_with_agent`, asserts `reconstruct_host_spec_from_log`
/// sees the persisted spec, creates a session, prompts, stops the runtime,
/// calls `src/orchestration.rs::resume`, and asserts the resumed runtime is
/// Ready with a new host_id under the same host_key.
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
/// (same host_key, same host_id, status Ready) without spinning up a
/// new runtime — the resume helper's short-circuit path is taken.
///
/// Invariant proven: **Orchestration idempotent on live runtimes** — calling
/// `resume` against a runtime that's already serving traffic is a no-op, not
/// a cold-start. This is the retry-safety property: a subscriber that sees
/// the same "needs to advance" event twice and calls `resume` twice does not
/// destabilize the runtime.
#[tokio::test]
async fn orchestration_resume_on_live_runtime_is_noop() -> Result<()> {
    let control_plane = ControlPlaneHarness::spawn(true).await?;

    let result = async {
        let runtime = control_plane
            .create_runtime_with_agent(
                "orchestration-resume-live-noop",
                &[managed_agent_suite::testy_load_bin().display().to_string()],
            )
            .await?;

        let session_id = create_session(&runtime.acp.url).await?;
        prompt_session(
            &runtime.acp.url,
            &session_id,
            "hello before the live-runtime resume",
        )
        .await?;

        let shared_state_url = control_plane.shared_state_url();
        let resumed_once = fireline_orchestration::resume(
            &control_plane.http,
            &control_plane.base_url,
            &shared_state_url,
            &session_id,
        )
        .await
        .context("first resume against live runtime must succeed")?;
        let resumed_twice = fireline_orchestration::resume(
            &control_plane.http,
            &control_plane.base_url,
            &shared_state_url,
            &session_id,
        )
        .await
        .context("second resume against live runtime must succeed")?;

        assert_eq!(
            resumed_once.host_key, runtime.host_key,
            "INVARIANT (Orchestration): resume on live runtime returns the same host_key"
        );
        assert_eq!(
            resumed_once.host_id, runtime.host_id,
            "INVARIANT (Orchestration): resume on live runtime does not spawn a new process"
        );
        assert_eq!(
            resumed_twice.host_id, runtime.host_id,
            "INVARIANT (Orchestration): repeated resume remains a no-op on the same runtime"
        );

        Ok(())
    }
    .await;

    control_plane.shutdown().await;
    result
}

/// Precondition: a session has been established on a live runtime.
///
/// Action: fire `resume(sessionId)` TWO TIMES CONCURRENTLY via
/// `tokio::join!`. The live-runtime short-circuit path in
/// `src/orchestration.rs` means both calls race through the shared
/// session index lookup and should return the same runtime without
/// creating a second process.
///
/// Observable evidence: both `resume` calls return descriptors with
/// the **same** `host_key` AND the **same** `host_id`, proving
/// no second runtime process was instantiated.
///
/// Invariant proven: **Orchestration concurrent-resume idempotency** —
/// two subscribers seeing the same "needs to advance" event and both
/// calling `resume` produce exactly one effective resumption. This is
/// the property the prior e2e spec claimed via a sequential check and
/// that the reviewer flagged as "proves nothing about the race" —
/// real concurrency via `tokio::join!` fixes the oracle.
#[tokio::test]
async fn orchestration_concurrent_resume_creates_single_runtime() -> Result<()> {
    let control_plane = ControlPlaneHarness::spawn(true).await?;

    let result = async {
        let runtime = control_plane
            .create_runtime_with_agent(
                "orchestration-concurrent-resume",
                &[managed_agent_suite::testy_load_bin().display().to_string()],
            )
            .await?;

        let session_id = create_session(&runtime.acp.url).await?;
        prompt_session(
            &runtime.acp.url,
            &session_id,
            "hello before the concurrent resume race",
        )
        .await?;

        let shared_state_url = control_plane.shared_state_url();
        let (first, second) = tokio::join!(
            fireline_orchestration::resume(
                &control_plane.http,
                &control_plane.base_url,
                &shared_state_url,
                &session_id,
            ),
            fireline_orchestration::resume(
                &control_plane.http,
                &control_plane.base_url,
                &shared_state_url,
                &session_id,
            ),
        );
        let first = first.context("first concurrent resume must succeed")?;
        let second = second.context("second concurrent resume must succeed")?;

        assert_eq!(
            first.host_key, runtime.host_key,
            "INVARIANT (Orchestration): concurrent resume A returns the same host_key"
        );
        assert_eq!(
            second.host_key, runtime.host_key,
            "INVARIANT (Orchestration): concurrent resume B returns the same host_key"
        );
        assert_eq!(
            first.host_id, runtime.host_id,
            "INVARIANT (Orchestration): concurrent resume A does not spawn a new runtime process"
        );
        assert_eq!(
            second.host_id, runtime.host_id,
            "INVARIANT (Orchestration): concurrent resume B does not spawn a new runtime process"
        );
        assert_eq!(
            first.host_id, second.host_id,
            "INVARIANT (Orchestration): both concurrent resumes observe the same runtime identity"
        );

        Ok(())
    }
    .await;

    control_plane.shutdown().await;
    result
}

/// Precondition: a deployment session exists on a runtime that has already
/// been stopped.
///
/// Action: deliver `deployment_wake_requested` to the Phase 6
/// `AlwaysOnDeploymentSubscriber`, which delegates straight to the existing
/// `resume(sessionId)` composition helper rather than implementing a second
/// wake state machine.
///
/// Observable evidence: the subscriber yields `sandbox_provisioned` for the
/// same logical deployment (`host_key`) with a fresh `runtime_id`, and the
/// resumed runtime still accepts `session/load` plus follow-up traffic on the
/// same session binding.
///
/// Invariant proven: **WakeOnStoppedChangesRuntimeId** and
/// **WakeOnStoppedPreservesSessionBinding** through the DurableSubscriber
/// profile surface.
#[tokio::test]
async fn durable_subscriber_always_on_wake_reprovisions_stopped_runtime() -> Result<()> {
    let control_plane = ControlPlaneHarness::spawn(true).await?;

    let result = async {
        let runtime = control_plane
            .create_runtime_with_agent(
                "always-on-deployment-stopped-runtime",
                &[managed_agent_suite::testy_load_bin().display().to_string()],
            )
            .await?;

        let session_id = create_session(&runtime.acp.url).await?;
        prompt_session(
            &runtime.acp.url,
            &session_id,
            "hello before always-on deployment wake",
        )
        .await?;

        let _stopped = control_plane.stop_runtime(&runtime.host_key).await?;
        let subscriber = AlwaysOnDeploymentSubscriber::new(
            reqwest::Client::new(),
            control_plane.base_url.clone(),
            control_plane.shared_state_url(),
        );

        let completion = expect_completed(
            subscriber
                .handle(DeploymentWakeRequested::new(SessionId::from(
                    session_id.clone(),
                )))
                .await,
            "DSV-13 RebuildRaceSafe: always-on deployment subscriber should reprovision a stopped runtime once for the replayed wake request",
        )?;
        let resumed = wait_for_runtime_descriptor_from_control_plane(
            &control_plane.http,
            &control_plane.base_url,
            &completion.runtime_key,
            &completion.runtime_id,
        )
        .await?;

        assert_eq!(
            completion.runtime_key, runtime.host_key,
            "INVARIANT (DurableSubscriber): deployment wake preserves deployment identity"
        );
        assert_eq!(
            completion.runtime_id, resumed.host_id,
            "INVARIANT (DurableSubscriber): completion reports the resumed runtime id"
        );
        assert_ne!(
            completion.runtime_id, runtime.host_id,
            "INVARIANT (DurableSubscriber): waking a stopped deployment rotates runtime identity"
        );

        load_session(&resumed.acp.url, &session_id).await.context(
            "INVARIANT (DurableSubscriber): resumed deployment must accept session/load",
        )?;
        prompt_session(
            &resumed.acp.url,
            &session_id,
            "hello after always-on deployment wake",
        )
        .await
        .context(
            "INVARIANT (DurableSubscriber): resumed deployment must serve follow-up prompts",
        )?;

        Ok(())
    }
    .await;

    control_plane.shutdown().await;
    result
}

/// Precondition: a deployment session exists on a runtime that is already
/// ready to serve traffic.
///
/// Action: deliver two `deployment_wake_requested` events concurrently through
/// the `AlwaysOnDeploymentSubscriber`.
///
/// Observable evidence: both completions resolve to the exact same runtime id
/// under the same deployment identity, proving wake requests converge on one
/// effective winner instead of churning the live runtime.
///
/// Invariant proven: **WakeOnReadyIsNoop** and **ConcurrentWakeSingleWinner**
/// through the DurableSubscriber profile surface.
#[tokio::test]
async fn durable_subscriber_always_on_concurrent_live_wake_returns_single_runtime() -> Result<()> {
    let control_plane = ControlPlaneHarness::spawn(true).await?;

    let result = async {
        let runtime = control_plane
            .create_runtime_with_agent(
                "always-on-deployment-live-runtime",
                &[managed_agent_suite::testy_load_bin().display().to_string()],
            )
            .await?;

        let session_id = create_session(&runtime.acp.url).await?;
        prompt_session(
            &runtime.acp.url,
            &session_id,
            "hello before concurrent always-on deployment wake",
        )
        .await?;

        let subscriber = AlwaysOnDeploymentSubscriber::new(
            reqwest::Client::new(),
            control_plane.base_url.clone(),
            control_plane.shared_state_url(),
        );

        let (first, second) = tokio::join!(
            subscriber.handle(DeploymentWakeRequested::new(SessionId::from(
                session_id.clone()
            ))),
            subscriber.handle(DeploymentWakeRequested::new(SessionId::from(
                session_id.clone()
            ))),
        );
        let first = expect_completed(
            first,
            "DSV-01 CompletionKeyUnique: first concurrent always-on wake should return the live runtime once",
        )?;
        let second = expect_completed(
            second,
            "DSV-01 CompletionKeyUnique: second concurrent always-on wake should return the same live runtime rather than minting a duplicate wake result",
        )?;

        assert_eq!(
            first.runtime_key, runtime.host_key,
            "INVARIANT (DurableSubscriber): first wake keeps the deployment identity stable"
        );
        assert_eq!(
            second.runtime_key, runtime.host_key,
            "INVARIANT (DurableSubscriber): second wake keeps the deployment identity stable"
        );
        assert_eq!(
            first.runtime_id, runtime.host_id,
            "INVARIANT (DurableSubscriber): wake on ready runtime is a no-op for caller A"
        );
        assert_eq!(
            second.runtime_id, runtime.host_id,
            "INVARIANT (DurableSubscriber): wake on ready runtime is a no-op for caller B"
        );
        assert_eq!(
            first.runtime_id, second.runtime_id,
            "INVARIANT (DurableSubscriber): concurrent wake requests converge on one runtime"
        );

        Ok(())
    }
    .await;

    control_plane.shutdown().await;
    result
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
    let spec =
        ManagedAgentHarnessSpec::new("orchestration-subscriber-loop").with_topology(topology);
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

        let prompt_outcome = tokio::time::timeout(std::time::Duration::from_secs(15), prompt_task)
            .await
            .context("prompt task did not complete within the post-approval window")?;
        prompt_outcome.context("prompt task panicked")?.context(
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
