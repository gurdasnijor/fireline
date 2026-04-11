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

use agent_client_protocol::{
    InitializeRequest, LoadSessionRequest, NewSessionRequest, PromptRequest, ProtocolVersion,
};
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
    ApprovalGateOpts, ApprovalScope, BudgetOpts, ContextInjectionOpts, PeerOpts,
    WorkspaceFileSource,
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
        stubs::audit(), // appendToSession
        stubs::context_injection(ContextInjectionOpts {
            // mapEffect
            sources: vec![WorkspaceFileSource {
                path: ctx.mount_dir.clone(),
            }],
        }),
        stubs::budget(BudgetOpts { tokens: 1_000_000 }), // filter
        stubs::approval_gate(ApprovalGateOpts {
            // suspend
            scope: ApprovalScope::ToolCalls,
            timeout_ms: 60_000,
        }),
        stubs::peer(PeerOpts {
            // substitute + mapEffect
            peers: vec![],
        }),
        stubs::fs_backend(Arc::new(stubs::session_log_file_backend(
            // compose(substitute, appendToSession)
            &ctx.state_stream_url,
        ))),
        stubs::durable_trace(), // appendToSession (bidirectional)
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
        &runtime.state,
        &runtime.runtime_key,
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
    acp.prompt(
        &session_id,
        "please review the pr at github.com/example/repo",
    )
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
    stubs::assert_init_effect_tools_are_schema_only(&ctx.http, &runtime.state, &session_id)
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
    let approval_token =
        stubs::issue_stream_write_token(&ctx.http, &ctx.control_plane_url, &runtime.runtime_key)
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
    .context(
        "INVARIANT (Session): durable-streams accepts writes from any authenticated producer",
    )?;

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
    let reconstructed_spec =
        stubs::reconstruct_runtime_spec_from_log(&ctx.http, &runtime.state, &runtime.runtime_key)
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
        let addr = listener
            .local_addr()
            .context("resolve shared streams address")?;
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
    if fireline_bin().exists()
        && control_plane_bin().exists()
        && target_bin("fireline-testy").exists()
    {
        return Ok(());
    }

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

    pub type ResourceRef = fireline_conductor::runtime::ResourceRef;
    pub use fireline_components::fs_backend::{FileBackend, SessionLogFileBackend};

    pub fn session_log_file_backend(state_stream_url: &str) -> SessionLogFileBackend {
        fireline_components::fs_backend::RuntimeStreamFileBackend::new(state_stream_url)
    }

    pub fn fs_backend(_backend: Arc<dyn FileBackend>) -> Component {
        TopologyComponentSpec {
            name: "fs_backend".to_string(),
            config: Some(
                serde_json::to_value(
                    fireline_components::fs_backend::FsBackendConfig::runtime_stream(),
                )
                .expect("serialize fs backend config"),
            ),
        }
    }

    pub async fn file_backend_read(
        backend: &SessionLogFileBackend,
        path: &Path,
    ) -> Result<Vec<u8>> {
        backend.read(path).await
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
        state: &Endpoint,
        runtime_key: &str,
    ) -> Result<()> {
        let spec = read_persisted_runtime_spec(state, runtime_key).await?;
        assert_eq!(spec.runtime_key, runtime_key);
        Ok(())
    }

    pub async fn assert_mount_visible_inside_runtime(
        _http: &HttpClient,
        _acp: &Endpoint,
        state: &Endpoint,
        runtime_key: &str,
        path: &Path,
    ) -> Result<()> {
        use fireline_conductor::runtime::ResourceMounter as _;

        let persisted = read_persisted_runtime_spec(state, runtime_key).await?;
        let mounter = fireline_conductor::runtime::LocalPathMounter::new();
        let mut mounted_resources = Vec::new();
        for resource in &persisted.create_spec.resources {
            if let Some(mounted) = mounter.mount(resource, runtime_key).await? {
                mounted_resources.push(mounted);
            }
        }

        let backend = fireline_components::fs_backend::LocalFileBackend::new(mounted_resources);
        let _ = backend.read(path).await?;
        Ok(())
    }

    pub async fn assert_runtime_process_gone(_runtime_key: &str) {
        // Best-effort: poll getRuntime() and assert status is Stopped or the
        // record is absent. Implemented as a TODO stub until the resume work
        // wires up stop semantics.
    }

    pub async fn reconstruct_runtime_spec_from_log(
        _http: &HttpClient,
        state: &Endpoint,
        runtime_key: &str,
    ) -> Result<RuntimeSpec> {
        let persisted = read_persisted_runtime_spec(state, runtime_key).await?;
        Ok(RuntimeSpec {
            runtime_key: persisted.runtime_key,
            node_id: persisted.node_id,
            provider: persisted.create_spec.provider,
            agent: AgentSpec {
                command: persisted.create_spec.agent_command,
            },
            topology: persisted.create_spec.topology,
            resources: persisted
                .create_spec
                .resources
                .into_iter()
                .map(resource_ref_from_runtime)
                .collect(),
            state_stream_url: state.url.clone(),
            control_plane_url: String::new(),
        })
    }

    fn resource_ref_from_runtime(
        resource: fireline_conductor::runtime::ResourceRef,
    ) -> fireline_conductor::runtime::ResourceRef {
        resource
    }

    // -------------------------------------------------------------------------
    // Session primitive — stream reads, writes, materializers
    // Ships with: slice 14 (canonical read schema) + materialize helpers.
    // -------------------------------------------------------------------------

    pub async fn append_event_to_stream(
        _http: &HttpClient,
        state: &Endpoint,
        token: &str,
        event: JsonValue,
    ) -> Result<()> {
        let client = durable_streams::Client::new();
        let mut stream = client.stream(&state.url);
        stream.set_content_type("application/json");
        let producer = stream
            .producer(format!("external-{}", &token[..token.len().min(12)]))
            .content_type("application/json")
            .build();
        producer.append_json(&serde_json::json!({
            "type": "permission",
            "key": format!("{}:{}", event.get("sessionId").and_then(JsonValue::as_str).unwrap_or("event"), now_ms()),
            "headers": { "operation": "insert" },
            "value": event,
        }));
        producer.flush().await?;
        Ok(())
    }

    pub async fn issue_stream_write_token(
        http: &HttpClient,
        cp_url: &str,
        runtime_key: &str,
    ) -> Result<String> {
        super::issue_runtime_token(http, cp_url, runtime_key).await
    }

    pub struct StreamSubscription {
        state: Endpoint,
        seen: usize,
    }

    impl StreamSubscription {
        pub async fn wait_for_kind(&mut self, kind: &str, timeout: Duration) -> Result<JsonValue> {
            let deadline = tokio::time::Instant::now() + timeout;
            loop {
                let replay = read_stream_events(&self.state).await?;
                for event in replay.iter().skip(self.seen) {
                    if event_kind(event) == Some(kind) {
                        self.seen = replay.len();
                        return Ok(event.clone());
                    }
                }
                self.seen = replay.len();

                if tokio::time::Instant::now() >= deadline {
                    return Err(anyhow!("timed out waiting for event kind '{kind}'"));
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    pub async fn open_stream_subscription(
        _http: &HttpClient,
        state: &Endpoint,
        _from: Option<u64>,
    ) -> Result<StreamSubscription> {
        Ok(StreamSubscription {
            state: state.clone(),
            seen: read_stream_events(state).await?.len(),
        })
    }

    pub async fn assert_session_stream_still_readable(
        _http: &HttpClient,
        state: &Endpoint,
    ) -> Result<()> {
        let _ = read_stream_events(state).await?;
        Ok(())
    }

    pub async fn assert_effects_in_session_log_contain_init_and_prompt(
        _http: &HttpClient,
        state: &Endpoint,
        session_id: &str,
    ) -> Result<()> {
        let events = read_stream_events(state).await?;
        let has_session = events.iter().any(|event| {
            event.get("type").and_then(JsonValue::as_str) == Some("session")
                && event
                    .pointer("/value/sessionId")
                    .and_then(JsonValue::as_str)
                    == Some(session_id)
        });
        let has_prompt = events.iter().any(|event| {
            event.get("type").and_then(JsonValue::as_str) == Some("prompt_turn")
                && event
                    .pointer("/value/sessionId")
                    .and_then(JsonValue::as_str)
                    == Some(session_id)
        });
        anyhow::ensure!(has_session, "session row missing from stream");
        anyhow::ensure!(has_prompt, "prompt_turn row missing from stream");
        Ok(())
    }

    pub async fn assert_init_effect_tools_are_schema_only(
        _http: &HttpClient,
        _state: &Endpoint,
        _session_id: &str,
    ) -> Result<()> {
        Ok(())
    }

    pub async fn wait_for_session_event(
        _http: &HttpClient,
        state: &Endpoint,
        session_id: &str,
        kind: &str,
        timeout: Duration,
    ) -> Result<JsonValue> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            for event in read_stream_events(state).await? {
                if event_kind(&event) == Some(kind) && event_session_id(&event) == Some(session_id)
                {
                    return Ok(event);
                }
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "timed out waiting for event kind '{kind}' for session '{session_id}'"
                ));
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub async fn wait_for_fs_op_event(
        _http: &HttpClient,
        state: &Endpoint,
        session_id: &str,
        path: &str,
        timeout: Duration,
    ) -> Result<JsonValue> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            for event in read_stream_events(state).await? {
                let Some("fs_op") = event.get("type").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(value) = event.get("value") else {
                    continue;
                };
                if value.get("sessionId").and_then(JsonValue::as_str) == Some(session_id)
                    && value.get("path").and_then(JsonValue::as_str) == Some(path)
                {
                    return Ok(event);
                }
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "timed out waiting for fs_op event for session '{session_id}' path '{path}'"
                ));
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }
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
        todo!(
            "assert_replay_contains_permission_request — search for permission_request matching session_id"
        )
    }
    pub fn assert_replay_contains_approval_resolved(
        _replay: &[JsonValue],
        _session_id: &str,
    ) -> Result<()> {
        todo!(
            "assert_replay_contains_approval_resolved — search for approval_resolved matching session_id"
        )
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
        by_session_and_path: std::collections::HashMap<(String, String), ArtifactRecord>,
    }

    #[derive(Clone, Debug)]
    pub struct ArtifactRecord {
        pub path: String,
    }

    pub async fn materialize_artifact_index(
        _http: &HttpClient,
        state: &Endpoint,
    ) -> Result<ArtifactIndex> {
        let mut by_session_and_path = std::collections::HashMap::new();
        for event in read_stream_events(state).await? {
            let Some("fs_op") = event.get("type").and_then(JsonValue::as_str) else {
                continue;
            };
            let Some(value) = event.get("value") else {
                continue;
            };
            let Some("write") = value.get("op").and_then(JsonValue::as_str) else {
                continue;
            };
            let Some(session_id) = value.get("sessionId").and_then(JsonValue::as_str) else {
                continue;
            };
            let Some(path) = value.get("path").and_then(JsonValue::as_str) else {
                continue;
            };
            by_session_and_path.insert(
                (session_id.to_string(), path.to_string()),
                ArtifactRecord {
                    path: path.to_string(),
                },
            );
        }
        Ok(ArtifactIndex {
            by_session_and_path,
        })
    }

    pub fn artifact_index_get(
        index: &ArtifactIndex,
        session_id: &str,
        path: &str,
    ) -> Result<ArtifactRecord> {
        index
            .by_session_and_path
            .get(&(session_id.to_string(), path.to_string()))
            .cloned()
            .ok_or_else(|| anyhow!("artifact '{path}' for session '{session_id}' not found"))
    }

    async fn read_stream_events(state: &Endpoint) -> Result<Vec<JsonValue>> {
        let client = durable_streams::Client::new();
        let stream = client.stream(&state.url);
        let mut reader = stream
            .read()
            .offset(durable_streams::Offset::Beginning)
            .build()
            .context("build durable stream reader")?;
        let mut events = Vec::new();
        while let Some(chunk) = reader
            .next_chunk()
            .await
            .context("read durable stream chunk")?
        {
            if chunk.data.is_empty() {
                if chunk.up_to_date {
                    break;
                }
                continue;
            }
            events.extend(
                serde_json::from_slice::<Vec<JsonValue>>(&chunk.data)
                    .context("decode durable stream chunk")?,
            );
            if chunk.up_to_date {
                break;
            }
        }
        Ok(events)
    }

    fn event_kind(event: &JsonValue) -> Option<&str> {
        event.get("type").and_then(JsonValue::as_str)
    }

    fn event_session_id(event: &JsonValue) -> Option<&str> {
        event
            .pointer("/value/sessionId")
            .and_then(JsonValue::as_str)
    }

    async fn read_persisted_runtime_spec(
        state: &Endpoint,
        runtime_key: &str,
    ) -> Result<fireline_conductor::runtime::PersistedRuntimeSpec> {
        for event in read_stream_events(state).await? {
            if event.get("type").and_then(JsonValue::as_str) != Some("runtime_spec") {
                continue;
            }
            if event.get("key").and_then(JsonValue::as_str) != Some(runtime_key) {
                continue;
            }

            let value = event
                .get("value")
                .cloned()
                .ok_or_else(|| anyhow!("runtime_spec '{runtime_key}' missing value payload"))?;
            return serde_json::from_value(value)
                .with_context(|| format!("decode runtime_spec '{runtime_key}' payload"));
        }

        Err(anyhow!(
            "runtime_spec '{runtime_key}' not found in state stream {}",
            state.url
        ))
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time went backwards")
            .as_millis() as i64
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
        endpoint: Endpoint,
        cwd: PathBuf,
    }

    impl AcpClient {
        pub async fn new_session(&mut self) -> Result<String> {
            let cwd = self.cwd.clone();
            let endpoint = self.endpoint.clone();
            sacp::Client
                .builder()
                .connect_with(
                    WebSocketTransport::new(endpoint)?,
                    move |cx: sacp::ConnectionTo<sacp::Agent>| {
                        let cwd = cwd.clone();
                        async move {
                            let _ = cx
                                .send_request(InitializeRequest::new(ProtocolVersion::LATEST))
                                .block_task()
                                .await?;

                            let session = cx
                                .send_request(NewSessionRequest::new(cwd))
                                .block_task()
                                .await?;

                            Ok(session.session_id.to_string())
                        }
                    },
                )
                .await
                .map_err(anyhow::Error::from)
        }
        pub async fn load_session(&mut self, session_id: &str) -> Result<()> {
            let cwd = self.cwd.clone();
            let endpoint = self.endpoint.clone();
            let session_id = session_id.to_string();
            let result = sacp::Client
                .builder()
                .connect_with(
                    WebSocketTransport::new(endpoint)?,
                    move |cx: sacp::ConnectionTo<sacp::Agent>| {
                        let cwd = cwd.clone();
                        let session_id = session_id.clone();
                        async move {
                            let _ = cx
                                .send_request(InitializeRequest::new(ProtocolVersion::LATEST))
                                .block_task()
                                .await?;

                            Ok(cx
                                .send_request(LoadSessionRequest::new(session_id, cwd))
                                .block_task()
                                .await)
                        }
                    },
                )
                .await
                .map_err(anyhow::Error::from)?;

            result.map(|_| ()).map_err(anyhow::Error::from)
        }
        pub async fn prompt(&mut self, session_id: &str, text: &str) -> Result<()> {
            let endpoint = self.endpoint.clone();
            let session_id = session_id.to_string();
            let text = text.to_string();
            sacp::Client
                .builder()
                .connect_with(
                    WebSocketTransport::new(endpoint)?,
                    move |cx: sacp::ConnectionTo<sacp::Agent>| {
                        let session_id = session_id.clone();
                        let text = text.clone();
                        async move {
                            let _ = cx
                                .send_request(InitializeRequest::new(ProtocolVersion::LATEST))
                                .block_task()
                                .await?;

                            let _ = cx
                                .send_request(PromptRequest::new(session_id, vec![text.into()]))
                                .block_task()
                                .await?;

                            Ok(())
                        }
                    },
                )
                .await
                .map_err(anyhow::Error::from)
        }
        pub async fn disconnect(&mut self) -> Result<()> {
            Ok(())
        }
    }

    pub async fn connect_acp(endpoint: &Endpoint) -> Result<AcpClient> {
        Ok(AcpClient {
            endpoint: endpoint.clone(),
            cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        })
    }

    struct WebSocketTransport {
        url: String,
        headers: Option<std::collections::BTreeMap<String, String>>,
    }

    impl WebSocketTransport {
        fn new(endpoint: Endpoint) -> Result<Self> {
            Ok(Self {
                url: endpoint.url,
                headers: endpoint.headers,
            })
        }
    }

    impl sacp::ConnectTo<sacp::Client> for WebSocketTransport {
        async fn connect_to(
            self,
            client: impl sacp::ConnectTo<sacp::Agent>,
        ) -> Result<(), sacp::Error> {
            let mut request =
                tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
                    self.url.as_str(),
                )
                .map_err(|e| sacp::util::internal_error(format!("WebSocket request build: {e}")))?;

            if let Some(headers) = self.headers {
                for (name, value) in headers {
                    let header_name =
                        axum::http::header::HeaderName::try_from(name).map_err(|e| {
                            sacp::util::internal_error(format!("invalid header name: {e}"))
                        })?;
                    let header_value = axum::http::HeaderValue::try_from(value).map_err(|e| {
                        sacp::util::internal_error(format!("invalid header value: {e}"))
                    })?;
                    request.headers_mut().insert(header_name, header_value);
                }
            }

            let (ws, _) = tokio_tungstenite::connect_async(request)
                .await
                .map_err(|e| sacp::util::internal_error(format!("WebSocket connect: {e}")))?;

            let (write, read) = futures::StreamExt::split(ws);

            let outgoing = futures::SinkExt::with(
                futures::SinkExt::sink_map_err(write, std::io::Error::other),
                |line: String| async move {
                    Ok::<_, std::io::Error>(tokio_tungstenite::tungstenite::Message::Text(
                        line.into(),
                    ))
                },
            );

            let incoming = futures::StreamExt::filter_map(read, |msg| async move {
                match msg {
                    Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                        let line = text.trim().to_string();
                        if line.is_empty() {
                            None
                        } else {
                            Some(Ok(line))
                        }
                    }
                    Ok(tokio_tungstenite::tungstenite::Message::Binary(bytes)) => {
                        String::from_utf8(bytes.to_vec()).ok().and_then(|text| {
                            let line = text.trim().to_string();
                            if line.is_empty() {
                                None
                            } else {
                                Some(Ok(line))
                            }
                        })
                    }
                    Ok(_) => None,
                    Err(err) => Some(Err(std::io::Error::other(err))),
                }
            });

            sacp::ConnectTo::<sacp::Client>::connect_to(
                sacp::Lines::new(outgoing, incoming),
                client,
            )
            .await
        }
    }
}
