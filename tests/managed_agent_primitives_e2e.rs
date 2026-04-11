//! # Managed-Agent Primitive E2E Validation (Executable Spec)
//!
//! This test is the **executable specification** for Fireline's implementation of
//! Anthropic's six managed-agent primitives: Session, Orchestration, Harness,
//! Sandbox, Resources, and Tools.
//!
//! See `docs/explorations/managed-agents-mapping.md` for the primitive definitions
//! and `docs/fireline-one-pager.md` for the short version.
//!
//! ## Status: CURRENTLY FAILING (by design)
//!
//! This test is marked `#[ignore]` because it depends on implementation work that
//! has not yet landed. The `stubs` module at the bottom defines placeholders for
//! every missing API — each stub is a `todo!()` that panics at runtime with a
//! description of which slice needs to ship. The test compiles today, which means
//! it reads as a spec. It panics when run, which means the progress toward
//! "substrate is validated" is directly visible as each `todo!()` gets replaced
//! by the real implementation.
//!
//! Known missing pieces (each is a stub below):
//!
//! - `resume(sessionId)` — the Orchestration composition helper; ships with the
//!   TS API surface + slice 14's durable `runtimeSpec` persistence
//! - `FsBackendComponent` + `LocalFileBackend` + `SessionLogFileBackend` — ships
//!   with slice 15 (Resources)
//! - `ResourceMounter` trait + `LocalPathMounter` — ships with slice 15
//! - Durable `runtimeSpec` persistence as a Session event — ships with slice 14
//! - `ApprovalGateComponent::rebuild_from_log` — ships with slice 16 (the first
//!   worked Orchestration-composition consumer)
//! - `emit_event_external` — the convenience wrapper for writing an event to the
//!   durable stream from outside the runtime; currently durable-streams accepts
//!   direct HTTP POSTs, this is just a small helper
//!
//! As implementations land, the corresponding `todo!()` in the stubs module gets
//! replaced with a real call and the test advances one phase. When the last
//! `todo!()` is gone, the test passes and the substrate is validated against every
//! contract the Anthropic managed-agents post defines.
//!
//! ## What this test validates
//!
//! For each of the six primitives, the test exercises the invariant(s) that
//! Anthropic's post names as the contract the primitive must satisfy:
//!
//! - **Session** — append-only, idempotent, replayable from any offset, consumable
//!   from any authenticated producer (writes are not gated on being the runtime)
//! - **Orchestration** — `resume(sessionId)` advances a session from any dormant
//!   state (including killed runtime), retries are idempotent, cold-start from
//!   stored spec produces a semantically equivalent session
//! - **Harness** — every effect the agent yields is visible in the Session log,
//!   conductor combinators compose in order, `ApprovalGateComponent` can suspend
//!   an effect and rebuild its pending state from the log after runtime death
//! - **Sandbox** — `provision()` once, execute many times via ACP, stop + recreate
//!   against the same spec yields a runtime that can `session/load` the same session
//! - **Resources** — physical mounts are visible to shell operations; ACP fs
//!   interception routes `fs/write_text_file` to a backend and captures the write
//!   as a Session event; `SessionLogFileBackend` supports cross-runtime reads
//! - **Tools** — tools registered via conductor components surface on the init
//!   Effect; the Tools contract is schema-only and transport-agnostic
//!
//! Plus cross-primitive composition invariants:
//!
//! - Orchestration ∘ Session — resume after crash sees every event emitted before
//!   the crash
//! - Sandbox ∘ Resources — mounts exist before the agent's first tool call
//! - Harness ∘ Session — no effect bypasses the durable log
//! - Tools ∘ Harness — tool registrations are visible on the init Effect
//!
//! ## How to run (once passing)
//!
//! ```sh
//! cargo test -p fireline --test managed_agent_primitives_e2e -- --ignored --nocapture
//! ```
//!
//! ## Where to look when it fails
//!
//! The test panics with a descriptive message pointing at the missing piece. Search
//! this file for `todo!(` to see the implementation punch list. Each `todo!()` cites
//! the slice that needs to ship for that piece to land.

#![allow(unused_imports, dead_code, unused_variables)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use axum::Router;
use fireline_conductor::runtime::{
    CreateRuntimeSpec, Endpoint, RuntimeDescriptor, RuntimeHost, RuntimeProviderKind,
    RuntimeProviderRequest, RuntimeStatus,
};
use fireline_conductor::topology::{TopologyComponentSpec, TopologySpec};
use reqwest::Client as HttpClient;
use serde_json::Value as JsonValue;
use tokio::process::{Child, Command};
use tokio::sync::oneshot;
use uuid::Uuid;

use stubs::{
    ApprovalGateOpts, ApprovalScope, BudgetOpts, ContextInjectionOpts, PeerOpts, WorkspaceFileSource,
};

// =============================================================================
// TEST ENTRY POINT
// =============================================================================

/// Full managed-agent primitive validation.
///
/// Walks the entire managed-agent lifecycle end to end, asserting every primitive
/// contract and every cross-primitive composition invariant along the way. Phases
/// are clearly marked; each phase cites which primitive(s) it validates.
#[tokio::test]
#[ignore = "executable spec — currently failing pending implementation; see module docs and todo!() list"]
async fn fireline_satisfies_managed_agent_primitives() -> Result<()> {
    let ctx = TestContext::spawn().await?;

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 1: Topology composition
    // Validates: Harness primitive (combinator algebra), Tools primitive (tool
    // registration as a component). Every one of the seven combinators is
    // represented at least once in this topology.
    // ─────────────────────────────────────────────────────────────────────────

    let topology = stubs::compose(vec![
        stubs::audit(),                                                // appendToSession
        stubs::context_injection(ContextInjectionOpts {                // mapEffect
            sources: vec![WorkspaceFileSource { path: ctx.mount_dir.clone() }],
        }),
        stubs::budget(BudgetOpts { tokens: 1_000_000 }),               // filter
        stubs::approval_gate(ApprovalGateOpts {                        // suspend
            scope: ApprovalScope::ToolCalls,
            timeout_ms: 60_000,
        }),
        stubs::peer(PeerOpts {                                         // substitute + mapEffect
            peers: vec![],
        }),
        stubs::fs_backend(Arc::new(stubs::session_log_file_backend(   // compose(substitute, appendToSession)
            &ctx.state_stream_url,
        ))),
        stubs::durable_trace(),                                        // appendToSession (bidirectional)
    ]);

    assert!(
        stubs::topology_component_count(&topology) >= 7,
        "INVARIANT (Harness): topology should contain all seven combinator kinds"
    );

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 2: Resources — physical mount at provision time
    // Validates: Resources primitive (physical half). Tests that a LocalPathMounter
    // makes the mount_path visible inside the runtime before the agent starts.
    // ─────────────────────────────────────────────────────────────────────────

    std::fs::write(
        ctx.mount_dir.join("hello.txt"),
        "hello from the managed-agent e2e spec",
    )
    .context("seed workspace file")?;

    let resources = vec![stubs::ResourceRef::LocalPath {
        path: ctx.mount_dir.clone(),
        mount_path: PathBuf::from("/work"),
    }];

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 3: Provision (Sandbox primitive — provision side)
    // Validates: Sandbox provision contract. After provision, the runtime is
    // reachable at its advertised ACP endpoint and the topology + resources are
    // active inside it.
    //
    // Also validates: Session primitive (indirect). The durable runtimeSpec is
    // persisted as a Session event so a future `resume` can cold-start from it.
    // ─────────────────────────────────────────────────────────────────────────

    let runtime = stubs::provision(stubs::RuntimeSpec {
        runtime_key: format!("runtime:e2e:{}", Uuid::new_v4()),
        node_id: "node:e2e".to_string(),
        provider: RuntimeProviderRequest::Local,
        agent: stubs::AgentSpec {
            command: vec![ctx.testy_bin.to_string_lossy().into_owned()],
        },
        topology: topology.clone(),
        resources: resources.clone(),
        state_stream_url: ctx.state_stream_url.clone(),
        control_plane_url: ctx.control_plane_url.clone(),
    })
    .await
    .context("provision: Sandbox provision contract")?;

    assert_eq!(
        runtime.status,
        RuntimeStatus::Ready,
        "INVARIANT (Sandbox): provision returns a ready runtime"
    );
    assert!(
        !runtime.acp.url.is_empty(),
        "INVARIANT (Sandbox): ready runtime advertises an ACP endpoint"
    );
    assert!(
        !runtime.state.url.is_empty(),
        "INVARIANT (Sandbox): ready runtime advertises a state endpoint"
    );

    stubs::assert_runtime_spec_persisted_in_session_log(
        &ctx.http,
        &runtime.state,
        &runtime.runtime_key,
    )
    .await
    .context("runtimeSpec must be durably persisted for future resume to cold-start")?;

    stubs::assert_mount_visible_inside_runtime(
        &ctx.http,
        &runtime.acp,
        Path::new("/work/hello.txt"),
    )
    .await
    .context("INVARIANT (Sandbox ∘ Resources): physical mount must exist before first effect")?;

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 4: Execute (Sandbox.execute + Harness I/O)
    // Validates: Sandbox execute contract, Harness combinator composition,
    // ApprovalGateComponent suspend behavior, BudgetComponent filter behavior,
    // ContextInjectionComponent mapEffect behavior.
    //
    // The testy agent is scripted to:
    //   1. Acknowledge the init Effect (surfaces the tools registered via topology)
    //   2. Emit a tool call that triggers the ApprovalGate (suspend)
    //   3. Wait for the approval
    //   4. After resume, write an artifact via fs/write_text_file
    //   5. Complete
    // ─────────────────────────────────────────────────────────────────────────

    let mut acp = stubs::connect_acp(&runtime.acp).await?;
    let session_id = acp.new_session().await?;

    // First prompt: triggers the testy agent's "tool call that needs approval" path.
    acp.prompt(&session_id, "please review the pr at github.com/example/repo")
        .await?;

    // Harness invariant: every effect the agent has yielded so far is visible in
    // the Session log, in order.
    stubs::assert_effects_in_session_log_contain_init_and_prompt(
        &ctx.http,
        &runtime.state,
        &session_id,
    )
    .await
    .context("INVARIANT (Harness ∘ Session): every effect is appended to the durable log")?;

    // Tools invariant: the init effect visible in the log carries the tool
    // registrations from PeerComponent, SmitheryComponent, and any fsBackend
    // tools. Schema-only, transport-agnostic.
    stubs::assert_init_effect_tools_are_schema_only(
        &ctx.http,
        &runtime.state,
        &session_id,
    )
    .await
    .context("INVARIANT (Tools): tool registrations are {name, description, input_schema}")?;

    // ApprovalGate invariant: the agent's tool call triggered a suspend combinator,
    // which wrote a PermissionRequest event to the Session log. The agent's
    // effect is held pending.
    stubs::wait_for_session_event(
        &ctx.http,
        &runtime.state,
        &session_id,
        "permission_request",
        Duration::from_secs(10),
    )
    .await
    .context("INVARIANT (Harness.suspend): ApprovalGate writes PermissionRequest")?;

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 5: Dormancy (simulate runtime death)
    // Validates: Session primitive (durability). The Session log is readable
    // after the runtime is gone.
    //
    // Also sets up the preconditions for the Orchestration composition: a session
    // with a pending PermissionRequest, no live runtime.
    // ─────────────────────────────────────────────────────────────────────────

    acp.disconnect().await?;
    stubs::stop_runtime(&ctx.http, &ctx.control_plane_url, &runtime.runtime_key).await?;

    stubs::assert_runtime_process_gone(&runtime.runtime_key).await;
    stubs::assert_session_stream_still_readable(&ctx.http, &runtime.state).await?;

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 6: External approval (Orchestration composition — producer side)
    // Validates: Session primitive (writes from any authenticated producer, not
    // only the runtime). This is the key reduction — orchestration is composable
    // because external processes can append to the durable log.
    // ─────────────────────────────────────────────────────────────────────────

    let mut subscriber =
        stubs::open_stream_subscription(&ctx.http, &runtime.state, /* from = live */ None).await?;

    // Simulate an operator resolving the approval out of band. The approval
    // service writes an Allow event directly to the durable stream via its own
    // bearer token — no live runtime needed.
    let approval_token = stubs::issue_stream_write_token(
        &ctx.http,
        &ctx.control_plane_url,
        &runtime.runtime_key,
    )
    .await?;

    stubs::append_event_to_stream(
        &ctx.http,
        &runtime.state,
        &approval_token,
        serde_json::json!({
            "kind": "approval_resolved",
            "sessionId": &session_id,
            "allow": true,
            "resolvedBy": "operator",
        }),
    )
    .await
    .context("INVARIANT (Session): durable-streams accepts writes from any authenticated producer")?;

    // Subscriber observes the Allow event — confirming bidirectional durable
    // stream access across unrelated processes.
    let allow_event = subscriber
        .wait_for_kind("approval_resolved", Duration::from_secs(5))
        .await
        .context("INVARIANT (Session.getEvents): subscribers see external appends")?;
    assert_eq!(
        allow_event.pointer("/sessionId").and_then(|v| v.as_str()),
        Some(session_id.as_str()),
        "approval event should reference the pending session"
    );

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 7: Resume (Orchestration composition — consumer side)
    // Validates: Orchestration primitive satisfied by composition. `resume` is:
    //   sessionStore.get(sid) → getRuntime(key) → provision(spec) if dormant
    //   → connectAcp → loadSession(sid)
    //
    // Also validates cross-primitive invariant Orchestration ∘ Session: cold-start
    // sees every event that was on the log when the runtime died, including the
    // PermissionRequest and the externally-written Allow.
    // ─────────────────────────────────────────────────────────────────────────

    let resumed = stubs::resume(&ctx.http, &ctx.control_plane_url, &session_id)
        .await
        .context("INVARIANT (Orchestration): resume from dormant state")?;

    assert_ne!(
        resumed.runtime_id, runtime.runtime_id,
        "INVARIANT (Orchestration): resume cold-starts a new runtime process"
    );
    assert_eq!(
        resumed.runtime_key, runtime.runtime_key,
        "INVARIANT (Orchestration): resumed runtime preserves the runtime_key identity"
    );
    assert_eq!(
        resumed.status,
        RuntimeStatus::Ready,
        "INVARIANT (Orchestration): resume returns a ready runtime"
    );

    // Concurrent-resume idempotency: calling resume again while the first is
    // still active should be safe (either returns the same runtime or a
    // semantically equivalent one).
    let second_resume = stubs::resume(&ctx.http, &ctx.control_plane_url, &session_id)
        .await
        .context("INVARIANT (Orchestration): resume is concurrent-safe and idempotent")?;
    assert_eq!(
        second_resume.runtime_key, resumed.runtime_key,
        "concurrent resume must be idempotent on runtime_key"
    );

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 8: Continue — ApprovalGate rebuild from log + artifact write
    // Validates: Harness suspend/resume via composition, Resources ACP fs
    // interception, Session as the artifact record.
    //
    // After resume, the ApprovalGateComponent rebuilds its pending state from the
    // Session log, sees the Allow event, releases the pause, and the agent
    // continues. The agent's next action is to write an artifact via
    // fs/write_text_file — which should be intercepted by the FsBackendComponent,
    // routed to the SessionLogFileBackend, and captured as an fs_op event.
    // ─────────────────────────────────────────────────────────────────────────

    let mut acp2 = stubs::connect_acp(&resumed.acp).await?;
    acp2.load_session(&session_id).await.context(
        "INVARIANT (Harness rebuild): session/load rebuilds harness state from durable log",
    )?;

    // The agent (testy, scripted) should now proceed past the released approval
    // gate and write an artifact via fs/write_text_file. Wait for the artifact
    // event to appear on the Session log.
    let artifact_path = "/work/output/report.md";
    stubs::wait_for_fs_op_event(
        &ctx.http,
        &runtime.state,
        &session_id,
        artifact_path,
        Duration::from_secs(10),
    )
    .await
    .context("INVARIANT (Resources): FsBackendComponent captures fs/write_text_file to the log")?;

    // Cross-runtime virtual filesystem: a separate process pointing at the same
    // Session stream via SessionLogFileBackend should see the artifact.
    let cross_backend = stubs::session_log_file_backend(&runtime.state.url);
    let cross_content = stubs::file_backend_read(&cross_backend, Path::new(artifact_path))
        .await
        .context(
            "INVARIANT (Resources): SessionLogFileBackend supports cross-runtime reads via shared stream",
        )?;
    assert!(
        !cross_content.is_empty(),
        "artifact content should be non-empty"
    );

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 9: Materializer queries (Session primitive — read side)
    // Validates: Session primitive's read contract via materializers. Folds over
    // the Session event log produce queryable state that downstream products
    // consume.
    // ─────────────────────────────────────────────────────────────────────────

    let session_index = stubs::materialize_session_store(&ctx.http, &runtime.state).await?;
    let session_record = stubs::session_store_get(&session_index, &session_id)
        .context("INVARIANT (Session): materializer surfaces durable session record")?;
    assert_eq!(session_record.session_id, session_id);

    let all_runtimes = stubs::session_store_list_runtimes(&session_index);
    assert!(
        all_runtimes
            .iter()
            .any(|k| k == &runtime.runtime_key || k == &resumed.runtime_key),
        "INVARIANT (Session): materializer surfaces all runtime records in the stream"
    );

    let artifact_index = stubs::materialize_artifact_index(&ctx.http, &runtime.state).await?;
    let artifact = stubs::artifact_index_get(&artifact_index, &session_id, artifact_path)
        .context("INVARIANT (Resources ∘ Session): artifact records surface via materializer")?;
    assert_eq!(artifact.path, artifact_path);

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 10: Full-replay invariant closing
    // Validates: Session primitive's replay-from-any-offset contract, and the
    // global invariant that the durable log is the source of truth (everything
    // observable in the live system must be reconstructible from the log alone).
    // ─────────────────────────────────────────────────────────────────────────

    let replay = stubs::replay_full_stream(&ctx.http, &runtime.state).await?;

    // Every effect the agent yielded must be present in the replay.
    stubs::assert_replay_contains_init_effect(&replay)
        .context("INVARIANT (Harness ∘ Session): init effect in replay")?;
    stubs::assert_replay_contains_permission_request(&replay, &session_id)
        .context("INVARIANT (Harness ∘ Session): suspend event in replay")?;
    stubs::assert_replay_contains_approval_resolved(&replay, &session_id)
        .context("INVARIANT (Session): external-append event in replay")?;
    stubs::assert_replay_contains_fs_write(&replay, &session_id, artifact_path)
        .context("INVARIANT (Resources ∘ Session): fs op in replay")?;

    // Idempotent replay: running the same replay twice produces the same derived
    // state (materializer folds are pure).
    let session_index_replay = stubs::materialize_session_store(&ctx.http, &runtime.state).await?;
    let session_record_replay = stubs::session_store_get(&session_index_replay, &session_id)
        .context("replay-time materialization of the session")?;
    assert_eq!(
        session_record, session_record_replay,
        "INVARIANT (Session): materializer fold is deterministic over the same log"
    );

    // Reconstructibility: given only the Session log plus the runtime_key, we
    // can reconstruct everything we need to resume — proves the log is the
    // source of truth.
    let reconstructed_spec = stubs::reconstruct_runtime_spec_from_log(
        &ctx.http,
        &runtime.state,
        &runtime.runtime_key,
    )
    .await
    .context("INVARIANT (Orchestration ∘ Session): runtimeSpec recoverable from log")?;
    assert_eq!(
        reconstructed_spec.runtime_key, runtime.runtime_key,
        "reconstructed spec identity matches"
    );

    // ─────────────────────────────────────────────────────────────────────────
    // Cleanup
    // ─────────────────────────────────────────────────────────────────────────

    acp2.disconnect().await.ok();
    stubs::stop_runtime(&ctx.http, &ctx.control_plane_url, &resumed.runtime_key)
        .await
        .ok();
    ctx.shutdown().await.ok();

    Ok(())
}

// =============================================================================
// TEST CONTEXT — spawns the durable-streams server, control plane, and tracks
// temp resources. This is boilerplate that can reuse helpers from
// tests/control_plane_push.rs.
// =============================================================================

struct TestContext {
    http: HttpClient,
    control_plane_url: String,
    state_stream_url: String,
    mount_dir: PathBuf,
    testy_bin: PathBuf,
    // Plus handles to spawned subprocesses that get killed in shutdown
    control_plane_child: Option<tokio::process::Child>,
    state_stream_server: Option<SharedStreamServer>,
}

impl TestContext {
    async fn spawn() -> Result<Self> {
        ensure_control_plane_binaries_built()?;

        let shared_streams = SharedStreamServer::spawn().await?;
        let runtime_registry_path = temp_path("fireline-managed-agent-runtimes");
        let peer_directory_path = temp_path("fireline-managed-agent-peers");
        let control_plane_url = format!("http://127.0.0.1:{}", reserve_port()?);
        let control_plane_child = spawn_control_plane(
            &control_plane_url,
            &runtime_registry_path,
            &peer_directory_path,
            &shared_streams.base_url,
        )
        .await?;

        let mount_dir = temp_path("fireline-managed-agent-mount");
        std::fs::create_dir_all(&mount_dir)
            .with_context(|| format!("create mount dir {}", mount_dir.display()))?;

        Ok(Self {
            http: HttpClient::new(),
            control_plane_url,
            state_stream_url: shared_streams.base_url.clone(),
            mount_dir,
            testy_bin: target_bin("fireline-testy"),
            control_plane_child: Some(control_plane_child),
            state_stream_server: Some(shared_streams),
        })
    }

    async fn shutdown(mut self) -> Result<()> {
        if let Some(mut child) = self.control_plane_child.take() {
            shutdown_process(&mut child).await;
        }
        if let Some(streams) = self.state_stream_server.take() {
            streams.shutdown().await;
        }
        let _ = std::fs::remove_dir_all(&self.mount_dir);
        Ok(())
    }
}

struct SharedStreamServer {
    base_url: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl SharedStreamServer {
    async fn spawn() -> Result<Self> {
        let router: Router = fireline::stream_host::build_stream_router(None)?;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .context("bind shared durable-streams test listener")?;
        let addr = listener.local_addr().context("resolve shared streams address")?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
        Ok(Self {
            base_url: format!("http://127.0.0.1:{}/v1/stream", addr.port()),
            shutdown_tx: Some(shutdown_tx),
            task,
        })
    }

    async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = self.task.await;
    }
}

fn ensure_control_plane_binaries_built() -> Result<()> {
    let status = std::process::Command::new("cargo")
        .args([
            "build",
            "--quiet",
            "-p",
            "fireline",
            "--bin",
            "fireline",
            "--bin",
            "fireline-testy",
            "-p",
            "fireline-control-plane",
            "--bin",
            "fireline-control-plane",
        ])
        .status()
        .context("build fireline test binaries")?;
    if !status.success() {
        return Err(anyhow!("failed to build fireline test binaries"));
    }
    Ok(())
}

fn target_bin(name: &str) -> PathBuf {
    let cargo_var = format!("CARGO_BIN_EXE_{name}");
    if let Some(path) = std::env::var_os(&cargo_var) {
        return PathBuf::from(path);
    }

    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(name)
}

fn fireline_bin() -> PathBuf {
    target_bin("fireline")
}

fn control_plane_bin() -> PathBuf {
    target_bin("fireline-control-plane")
}

fn temp_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()))
}

async fn spawn_control_plane(
    base_url: &str,
    runtime_registry_path: &PathBuf,
    peer_directory_path: &PathBuf,
    shared_stream_base_url: &str,
) -> Result<Child> {
    let port = base_url
        .rsplit(':')
        .next()
        .ok_or_else(|| anyhow!("missing control-plane port"))?;
    let mut command = Command::new(control_plane_bin());
    command
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port)
        .arg("--fireline-bin")
        .arg(fireline_bin())
        .arg("--runtime-registry-path")
        .arg(runtime_registry_path)
        .arg("--peer-directory-path")
        .arg(peer_directory_path)
        .arg("--prefer-push")
        .arg("--shared-stream-base-url")
        .arg(shared_stream_base_url)
        .arg("--startup-timeout-ms")
        .arg("20000")
        .arg("--stop-timeout-ms")
        .arg("10000")
        .arg("--heartbeat-scan-interval-ms")
        .arg("5000")
        .arg("--stale-timeout-ms")
        .arg("30000")
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let mut child = command.spawn().context("spawn fireline-control-plane")?;
    wait_for_http_ok(&format!("{base_url}/healthz"), &mut child).await?;
    Ok(child)
}

async fn issue_runtime_token(
    client: &reqwest::Client,
    base_url: &str,
    runtime_key: &str,
) -> Result<String> {
    let response = client
        .post(format!("{base_url}/v1/auth/runtime-token"))
        .json(&serde_json::json!({
            "runtimeKey": runtime_key,
            "scope": "runtime.write"
        }))
        .send()
        .await?
        .error_for_status()?;
    let payload = response.json::<serde_json::Value>().await?;
    payload
        .get("token")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .ok_or_else(|| anyhow!("missing runtime token"))
}

async fn wait_for_http_ok(url: &str, child: &mut Child) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(anyhow!(
                "control plane exited before becoming ready: {status}"
            ));
        }

        match reqwest::get(url).await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(_) | Err(_) if tokio::time::Instant::now() >= deadline => {
                return Err(anyhow!("timed out waiting for control plane at {url}"));
            }
            Ok(_) | Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

async fn shutdown_process(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }

    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
}

fn reserve_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .context("bind ephemeral port")?;
    Ok(listener.local_addr()?.port())
}

// =============================================================================
// STUBS — placeholders for APIs that don't exist yet. Each stub is a `todo!()`
// that panics at runtime with a description of which slice or piece needs to
// ship. As implementations land, replace the stub with the real import and
// delete the stub function.
// =============================================================================

mod stubs {
    use super::*;

    // -------------------------------------------------------------------------
    // Harness primitive — Component algebra
    // Ships with: `@fireline/client` + the Rust-side combinator layer that backs
    // it. The TS functional API proposal in
    // `docs/explorations/typescript-functional-api-proposal.md` has the shape.
    // -------------------------------------------------------------------------

    pub type Topology = TopologySpec;
    pub type Component = TopologyComponentSpec;

    pub fn compose(components: Vec<Component>) -> Topology {
        TopologySpec { components }
    }

    pub fn topology_component_count(t: &Topology) -> usize {
        t.components.len()
    }

    pub fn audit() -> Component {
        TopologyComponentSpec {
            name: "audit".to_string(),
            config: Some(serde_json::json!({
                "streamName": format!("managed-agent-audit-{}", Uuid::new_v4()),
            })),
        }
    }

    pub struct ContextInjectionOpts {
        pub sources: Vec<WorkspaceFileSource>,
    }
    pub struct WorkspaceFileSource {
        pub path: PathBuf,
    }
    pub fn context_injection(opts: ContextInjectionOpts) -> Component {
        TopologyComponentSpec {
            name: "context_injection".to_string(),
            config: Some(serde_json::json!({
                "sources": opts.sources.into_iter().map(|source| {
                    serde_json::json!({
                        "kind": "workspaceFile",
                        "path": source.path,
                    })
                }).collect::<Vec<_>>()
            })),
        }
    }

    pub struct BudgetOpts {
        pub tokens: u64,
    }
    pub fn budget(opts: BudgetOpts) -> Component {
        TopologyComponentSpec {
            name: "budget".to_string(),
            config: Some(serde_json::json!({
                "maxTokens": opts.tokens,
            })),
        }
    }

    pub enum ApprovalScope {
        ToolCalls,
    }
    pub struct ApprovalGateOpts {
        pub scope: ApprovalScope,
        pub timeout_ms: u64,
    }
    pub fn approval_gate(opts: ApprovalGateOpts) -> Component {
        let scope = match opts.scope {
            ApprovalScope::ToolCalls => "tool_calls",
        };
        TopologyComponentSpec {
            name: "approval_gate".to_string(),
            config: Some(serde_json::json!({
                "scope": scope,
                "timeoutMs": opts.timeout_ms,
            })),
        }
    }

    pub struct PeerOpts {
        pub peers: Vec<String>,
    }
    pub fn peer(opts: PeerOpts) -> Component {
        TopologyComponentSpec {
            name: "peer_mcp".to_string(),
            config: Some(serde_json::json!({
                "peers": opts.peers,
            })),
        }
    }

    pub fn durable_trace() -> Component {
        TopologyComponentSpec {
            name: "durable_trace".to_string(),
            config: None,
        }
    }

    // -------------------------------------------------------------------------
    // Resources primitive — FsBackendComponent + FileBackend
    // Ships with: slice 15 Resources refactor.
    // -------------------------------------------------------------------------

    #[derive(Clone)]
    pub enum ResourceRef {
        LocalPath { path: PathBuf, mount_path: PathBuf },
        GitRemote { repo_url: String, reference: Option<String>, mount_path: PathBuf },
        S3 { bucket: String, prefix: String, mount_path: PathBuf },
        Gcs { bucket: String, prefix: String, mount_path: PathBuf },
    }

    #[async_trait::async_trait]
    pub trait FileBackend: Send + Sync {
        async fn read(&self, path: &Path) -> Result<Vec<u8>>;
        async fn write(&self, path: &Path, content: &[u8]) -> Result<()>;
    }

    pub struct SessionLogFileBackend {
        _state_stream_url: String,
    }

    #[async_trait::async_trait]
    impl FileBackend for SessionLogFileBackend {
        async fn read(&self, _path: &Path) -> Result<Vec<u8>> {
            todo!(
                "SessionLogFileBackend::read — slice 15 Resources refactor. \
                 Should project fs_op events from the Session log into a \
                 file content map and return the latest content for the path."
            )
        }
        async fn write(&self, _path: &Path, _content: &[u8]) -> Result<()> {
            todo!(
                "SessionLogFileBackend::write — slice 15 Resources refactor. \
                 Should append an fs_op event to the Session log."
            )
        }
    }

    pub fn session_log_file_backend(state_stream_url: &str) -> SessionLogFileBackend {
        SessionLogFileBackend {
            _state_stream_url: state_stream_url.to_string(),
        }
    }

    pub fn fs_backend(_backend: Arc<dyn FileBackend>) -> Component {
        TopologyComponentSpec {
            name: "fs_backend".to_string(),
            config: None,
        }
    }

    pub async fn file_backend_read(_backend: &SessionLogFileBackend, _path: &Path) -> Result<Vec<u8>> {
        todo!(
            "file_backend_read — slice 15 Resources refactor. Trivial wrapper \
             around FileBackend::read for test ergonomics."
        )
    }

    // -------------------------------------------------------------------------
    // Sandbox primitive — provision + runtime lifecycle
    // Ships with: slices 13a/13b (already shipped) + slice 14 (durable
    // runtimeSpec persistence) + slice 15 (resources in spec).
    // -------------------------------------------------------------------------

    pub struct RuntimeSpec {
        pub runtime_key: String,
        pub node_id: String,
        pub provider: RuntimeProviderRequest,
        pub agent: AgentSpec,
        pub topology: Topology,
        pub resources: Vec<ResourceRef>,
        pub state_stream_url: String,
        pub control_plane_url: String,
    }

    pub struct AgentSpec {
        pub command: Vec<String>,
    }

    pub async fn provision(_spec: RuntimeSpec) -> Result<RuntimeDescriptor> {
        todo!(
            "provision — should call the control plane /v1/runtimes endpoint \
             with the CreateRuntimeSpec shape including resources and topology, \
             then wait for Ready status. The spec must be durably persisted as \
             a Session event so future resume() can cold-start from it. Ships \
             with slice 14 (runtimeSpec persistence) + slice 15 (resources \
             field)."
        )
    }

    pub async fn stop_runtime(_http: &HttpClient, _cp_url: &str, _runtime_key: &str) -> Result<()> {
        // Today this can be implemented via a POST to /v1/runtimes/{key}/stop.
        // Stubbed to keep the test compiling against the evolving surface.
        todo!(
            "stop_runtime — reuse the existing POST /v1/runtimes/{{key}}/stop \
             endpoint from the phase 1 control plane."
        )
    }

    pub async fn assert_runtime_spec_persisted_in_session_log(
        _http: &HttpClient,
        _state: &Endpoint,
        _runtime_key: &str,
    ) -> Result<()> {
        todo!(
            "assert_runtime_spec_persisted_in_session_log — queries the Session \
             log for a runtime_spec event matching runtime_key and asserts the \
             spec is fully present (agent, topology, resources). Closes the \
             slice 14 acceptance-bar item 'durable runtimeSpec persistence'."
        )
    }

    pub async fn assert_mount_visible_inside_runtime(
        _http: &HttpClient,
        _acp: &Endpoint,
        _path: &Path,
    ) -> Result<()> {
        todo!(
            "assert_mount_visible_inside_runtime — issues an fs/read_text_file \
             ACP request against the runtime and verifies the mounted file is \
             readable. Closes slice 15's LocalPathMounter acceptance bar."
        )
    }

    pub async fn assert_runtime_process_gone(_runtime_key: &str) {
        // Best-effort: poll getRuntime() and assert status is Stopped or the
        // record is absent. Implemented as a TODO stub until the resume work
        // wires up stop semantics.
    }

    pub async fn reconstruct_runtime_spec_from_log(
        _http: &HttpClient,
        _state: &Endpoint,
        _runtime_key: &str,
    ) -> Result<RuntimeSpec> {
        todo!(
            "reconstruct_runtime_spec_from_log — reads the durable Session log \
             and rebuilds the RuntimeSpec from the persisted runtime_spec event. \
             This is literally what resume() does internally; extracting it as a \
             helper lets the test assert the reconstructibility invariant \
             directly."
        )
    }

    // -------------------------------------------------------------------------
    // Session primitive — stream reads, writes, materializers
    // Ships with: slice 14 (canonical read schema) + materialize helpers.
    // -------------------------------------------------------------------------

    pub async fn append_event_to_stream(
        _http: &HttpClient,
        _state: &Endpoint,
        _token: &str,
        _event: JsonValue,
    ) -> Result<()> {
        todo!(
            "append_event_to_stream — POSTs an event to the durable-streams HTTP \
             endpoint with the external bearer token. Today this already works \
             via a direct HTTP call; the stub exists so the test has an ergonomic \
             helper. Implementation: reqwest POST {{state.url}}/v1/events with \
             Bearer {{token}}."
        )
    }

    pub async fn issue_stream_write_token(
        _http: &HttpClient,
        _cp_url: &str,
        _runtime_key: &str,
    ) -> Result<String> {
        todo!(
            "issue_stream_write_token — calls /v1/auth/runtime-token (already \
             exists from slice 13b) to mint a write-scoped bearer token for the \
             runtime's stream. The test uses this to simulate an external \
             approval service writing back to the stream."
        )
    }

    pub struct StreamSubscription {
        _placeholder: (),
    }

    impl StreamSubscription {
        pub async fn wait_for_kind(
            &mut self,
            _kind: &str,
            _timeout: Duration,
        ) -> Result<JsonValue> {
            todo!(
                "StreamSubscription::wait_for_kind — async iterator over SSE \
                 events from the durable stream; returns the first event whose \
                 kind matches. Slice 14 or earlier materializer helper."
            )
        }
    }

    pub async fn open_stream_subscription(
        _http: &HttpClient,
        _state: &Endpoint,
        _from: Option<u64>,
    ) -> Result<StreamSubscription> {
        todo!(
            "open_stream_subscription — opens an SSE subscription to the \
             durable-streams endpoint with an offset cursor. Trivial wrapper \
             around the existing durable-streams client."
        )
    }

    pub async fn assert_session_stream_still_readable(
        _http: &HttpClient,
        _state: &Endpoint,
    ) -> Result<()> {
        todo!(
            "assert_session_stream_still_readable — GET the durable stream with \
             a short offset range and assert the response is successful. Proves \
             the Session primitive survives runtime death."
        )
    }

    pub async fn assert_effects_in_session_log_contain_init_and_prompt(
        _http: &HttpClient,
        _state: &Endpoint,
        _session_id: &str,
    ) -> Result<()> {
        todo!(
            "assert_effects_in_session_log_contain_init_and_prompt — reads the \
             recent slice of the Session log and asserts that both an `init` \
             effect and the `prompt` effect we just sent are present. Closes the \
             Harness ∘ Session invariant: no effect bypasses the durable log."
        )
    }

    pub async fn assert_init_effect_tools_are_schema_only(
        _http: &HttpClient,
        _state: &Endpoint,
        _session_id: &str,
    ) -> Result<()> {
        todo!(
            "assert_init_effect_tools_are_schema_only — reads the init effect \
             from the log and asserts each tool has {{name, description, \
             input_schema}} and nothing else (no transport details, no \
             credentials). Closes the Tools invariant."
        )
    }

    pub async fn wait_for_session_event(
        _http: &HttpClient,
        _state: &Endpoint,
        _session_id: &str,
        _kind: &str,
        _timeout: Duration,
    ) -> Result<JsonValue> {
        todo!(
            "wait_for_session_event — polls or subscribes to the Session stream \
             and returns the first event of the given kind for the given \
             session. Trivial helper over open_stream_subscription."
        )
    }

    pub async fn wait_for_fs_op_event(
        _http: &HttpClient,
        _state: &Endpoint,
        _session_id: &str,
        _path: &str,
        _timeout: Duration,
    ) -> Result<JsonValue> {
        todo!(
            "wait_for_fs_op_event — similar to wait_for_session_event but \
             specifically filters for fs_op events with the given path. Closes \
             the Resources ∘ Session invariant."
        )
    }

    pub async fn replay_full_stream(
        _http: &HttpClient,
        _state: &Endpoint,
    ) -> Result<Vec<JsonValue>> {
        todo!(
            "replay_full_stream — reads the Session stream from offset 0 to \
             current and returns the ordered list of events. Used for the \
             full-replay invariant closing phase."
        )
    }

    pub fn assert_replay_contains_init_effect(_replay: &[JsonValue]) -> Result<()> {
        todo!("assert_replay_contains_init_effect — search the replay for an init effect")
    }
    pub fn assert_replay_contains_permission_request(
        _replay: &[JsonValue],
        _session_id: &str,
    ) -> Result<()> {
        todo!("assert_replay_contains_permission_request — search for permission_request matching session_id")
    }
    pub fn assert_replay_contains_approval_resolved(
        _replay: &[JsonValue],
        _session_id: &str,
    ) -> Result<()> {
        todo!("assert_replay_contains_approval_resolved — search for approval_resolved matching session_id")
    }
    pub fn assert_replay_contains_fs_write(
        _replay: &[JsonValue],
        _session_id: &str,
        _path: &str,
    ) -> Result<()> {
        todo!("assert_replay_contains_fs_write — search for fs_op events with the given path")
    }

    // -------------------------------------------------------------------------
    // Session primitive — materializers (read side)
    // -------------------------------------------------------------------------

    #[derive(Clone, PartialEq, Eq, Debug)]
    pub struct SessionRecord {
        pub session_id: String,
    }

    pub struct SessionIndex {
        _placeholder: (),
    }

    pub async fn materialize_session_store(
        _http: &HttpClient,
        _state: &Endpoint,
    ) -> Result<SessionIndex> {
        todo!(
            "materialize_session_store — runs the preset sessionStore \
             materializer over the Session stream and returns a queryable \
             SessionIndex. Ships with slice 14."
        )
    }

    pub fn session_store_get(_index: &SessionIndex, _session_id: &str) -> Result<SessionRecord> {
        todo!("session_store_get — SessionIndex::get, slice 14 materializer")
    }

    pub fn session_store_list_runtimes(_index: &SessionIndex) -> Vec<String> {
        todo!("session_store_list_runtimes — list distinct runtime_keys from the stream")
    }

    pub struct ArtifactIndex {
        _placeholder: (),
    }

    #[derive(Debug)]
    pub struct ArtifactRecord {
        pub path: String,
    }

    pub async fn materialize_artifact_index(
        _http: &HttpClient,
        _state: &Endpoint,
    ) -> Result<ArtifactIndex> {
        todo!(
            "materialize_artifact_index — folds fs_op events into an artifact \
             map. Falls out of the FsBackendComponent work in slice 15."
        )
    }

    pub fn artifact_index_get(
        _index: &ArtifactIndex,
        _session_id: &str,
        _path: &str,
    ) -> Result<ArtifactRecord> {
        todo!("artifact_index_get — lookup over the materialized artifact map")
    }

    // -------------------------------------------------------------------------
    // Orchestration composition helper — resume(sessionId)
    // Ships with: the @fireline/client TS API surface + a Rust equivalent for
    // test ergonomics.
    // -------------------------------------------------------------------------

    pub async fn resume(
        _http: &HttpClient,
        _cp_url: &str,
        _session_id: &str,
    ) -> Result<RuntimeDescriptor> {
        todo!(
            "resume — THE load-bearing composition helper. Does: (1) look up \
             session → runtime_key → runtimeSpec from the Session read surface \
             (slice 14), (2) check if the runtime is live via the control plane, \
             (3) if dormant, call provision() with the stored spec to cold-start, \
             (4) connectAcp to the new runtime, (5) call loadSession(session_id) \
             to rebuild ACP state via the existing LoadCoordinatorComponent. \
             Ten-line helper once all its dependencies land. See \
             managed-agents-mapping.md §2 for the full composition walkthrough."
        )
    }

    // -------------------------------------------------------------------------
    // ACP client — already exists in the workspace, stubbed here for test
    // ergonomics until we decide where the test harness helpers live.
    // -------------------------------------------------------------------------

    pub struct AcpClient {
        _placeholder: (),
    }

    impl AcpClient {
        pub async fn new_session(&mut self) -> Result<String> {
            todo!(
                "AcpClient::new_session — creates an ACP session and returns \
                 the session_id. Wrap existing ACP client helpers from \
                 tests/mesh_baseline.rs or similar."
            )
        }
        pub async fn load_session(&mut self, _session_id: &str) -> Result<()> {
            todo!(
                "AcpClient::load_session — issues ACP session/load; existing \
                 LoadCoordinatorComponent handles the rebuild."
            )
        }
        pub async fn prompt(&mut self, _session_id: &str, _text: &str) -> Result<()> {
            todo!("AcpClient::prompt — issues ACP session/prompt")
        }
        pub async fn disconnect(&mut self) -> Result<()> {
            Ok(())
        }
    }

    pub async fn connect_acp(_endpoint: &Endpoint) -> Result<AcpClient> {
        todo!(
            "connect_acp — opens an ACP WebSocket against the endpoint URL \
             carrying the endpoint's bearer token. Reuse helpers from existing \
             tests."
        )
    }
}
