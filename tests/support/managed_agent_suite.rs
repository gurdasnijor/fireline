#![allow(dead_code)]

use std::collections::BTreeSet;
use std::net::IpAddr;
use std::path::PathBuf;
use std::process::{Command as StdCommand, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use axum::Router;
use durable_streams::{Client as DsClient, Offset};
use fireline::bootstrap::{BootstrapConfig, BootstrapHandle, start};
use fireline::runtime_host::{RuntimeDescriptor, RuntimeStatus};
use fireline_components::fs_backend::{FsOpRecord, RuntimeStreamFileRecord};
use fireline_conductor::runtime::{
    LocalPathMounter, PersistedRuntimeSpec, ResourceMounter, ResourceRef,
};
use fireline_conductor::session::SessionRecord;
use fireline_conductor::topology::TopologySpec;
use futures::{SinkExt, StreamExt};
use serde_json::{Value as JsonValue, json};
use tokio::process::{Child, Command as TokioCommand};
use tokio::sync::oneshot;
use uuid::Uuid;

pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Primitive {
    Session,
    Orchestration,
    Harness,
    Sandbox,
    Resources,
    Tools,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SurfaceOwner {
    RustSubstrate,
    TypeScriptState,
    TypeScriptClient,
    CrossSurface,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContractCase {
    pub id: &'static str,
    pub primitive: Primitive,
    pub owner: SurfaceOwner,
    pub summary: &'static str,
}

const CONTRACT_INVENTORY: &[ContractCase] = &[
    ContractCase {
        id: "session.raw_log_replay",
        primitive: Primitive::Session,
        owner: SurfaceOwner::RustSubstrate,
        summary: "append-only durable stream, replayable from any offset",
    },
    ContractCase {
        id: "session.external_consumer",
        primitive: Primitive::Session,
        owner: SurfaceOwner::TypeScriptState,
        summary: "packages/state proves replay + catch-up as an external consumer",
    },
    ContractCase {
        id: "session.hot_vs_cold_surface",
        primitive: Primitive::Session,
        owner: SurfaceOwner::TypeScriptClient,
        summary: "client surface distinguishes ACP traffic from read-oriented state",
    },
    ContractCase {
        id: "sandbox.provision_execute",
        primitive: Primitive::Sandbox,
        owner: SurfaceOwner::RustSubstrate,
        summary: "runtime can be provisioned once and exercised repeatedly over ACP",
    },
    ContractCase {
        id: "harness.durable_trace",
        primitive: Primitive::Harness,
        owner: SurfaceOwner::RustSubstrate,
        summary: "effects are visible on the durable session log",
    },
    ContractCase {
        id: "harness.suspend_resume_rebuild",
        primitive: Primitive::Harness,
        owner: SurfaceOwner::RustSubstrate,
        summary: "components can suspend, persist, and rebuild on session/load",
    },
    ContractCase {
        id: "tools.schema_only",
        primitive: Primitive::Tools,
        owner: SurfaceOwner::RustSubstrate,
        summary: "tool registration is transport-agnostic schema, not transport state",
    },
    ContractCase {
        id: "orchestration.resume_helper",
        primitive: Primitive::Orchestration,
        owner: SurfaceOwner::TypeScriptClient,
        summary: "@fireline/client owns the resume(sessionId) composition helper",
    },
    ContractCase {
        id: "orchestration.subscriber_loop",
        primitive: Primitive::Orchestration,
        owner: SurfaceOwner::CrossSurface,
        summary: "subscriber loop spans durable streams, control plane, and client composition",
    },
    ContractCase {
        id: "resources.launch_spec",
        primitive: Primitive::Resources,
        owner: SurfaceOwner::CrossSurface,
        summary: "resources field spans TS launch spec plus Rust provider wiring",
    },
    ContractCase {
        id: "resources.fs_backend_artifacts",
        primitive: Primitive::Resources,
        owner: SurfaceOwner::CrossSurface,
        summary: "ACP fs interception writes artifact evidence readable via session materialization",
    },
];

pub(crate) fn contract_inventory() -> &'static [ContractCase] {
    CONTRACT_INVENTORY
}

pub(crate) fn covered_primitives() -> BTreeSet<Primitive> {
    CONTRACT_INVENTORY
        .iter()
        .map(|case| case.primitive)
        .collect()
}

pub(crate) fn temp_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()))
}

pub(crate) fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub(crate) fn fireline_bin() -> PathBuf {
    target_bin("fireline")
}

pub(crate) fn control_plane_bin() -> PathBuf {
    target_bin("fireline-control-plane")
}

pub(crate) fn testy_bin() -> PathBuf {
    target_bin("fireline-testy")
}

pub(crate) fn testy_load_bin() -> PathBuf {
    target_bin("fireline-testy-load")
}

pub(crate) fn testy_fs_bin() -> PathBuf {
    target_bin("fireline-testy-fs")
}

pub(crate) fn target_bin(name: &str) -> PathBuf {
    let cargo_var = format!("CARGO_BIN_EXE_{name}");
    if let Some(path) = std::env::var_os(&cargo_var) {
        return PathBuf::from(path);
    }

    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    repo_root().join("target").join(profile).join(name)
}

pub(crate) fn ensure_control_plane_binaries_built() -> Result<()> {
    let status = StdCommand::new("cargo")
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
        .context("build fireline control-plane test binaries")?;
    if !status.success() {
        return Err(anyhow!(
            "failed to build fireline control-plane test binaries"
        ));
    }
    Ok(())
}

pub(crate) struct LocalRuntimeHarness {
    handle: BootstrapHandle,
}

impl LocalRuntimeHarness {
    pub(crate) async fn spawn(name: &str) -> Result<Self> {
        let handle = start(BootstrapConfig {
            host: "127.0.0.1".parse::<IpAddr>()?,
            port: 0,
            name: name.to_string(),
            runtime_key: format!("runtime:{}", Uuid::new_v4()),
            node_id: "node:managed-agent-suite".to_string(),
            agent_command: vec![testy_bin().display().to_string()],
            mounted_resources: Vec::new(),
            state_stream: None,
            stream_storage: None,
            peer_directory_path: temp_path("fireline-managed-agent-peers"),
            control_plane_url: None,
            external_state_stream_url: None,
            topology: TopologySpec::default(),
        })
        .await?;

        Ok(Self { handle })
    }

    pub(crate) fn acp_url(&self) -> &str {
        &self.handle.acp_url
    }

    pub(crate) fn state_stream_url(&self) -> &str {
        &self.handle.state_stream_url
    }

    pub(crate) async fn prompt(&self, text: &str) -> Result<String> {
        yopo::prompt(
            WebSocketTransport {
                url: self.handle.acp_url.clone(),
            },
            text,
        )
        .await
        .map_err(anyhow::Error::from)
    }

    pub(crate) async fn wait_for_state_rows(
        &self,
        required_substrings: &[&str],
        timeout: Duration,
    ) -> Result<String> {
        wait_for_stream_rows(&self.handle.state_stream_url, required_substrings, timeout).await
    }

    pub(crate) async fn shutdown(self) -> Result<()> {
        self.handle.shutdown().await
    }
}

pub(crate) struct ControlPlaneHarness {
    pub http: reqwest::Client,
    pub base_url: String,
    pub runtime_registry_path: PathBuf,
    pub peer_directory_path: PathBuf,
    pub shared_state_stream_name: String,
    shared_stream_server: SharedStreamServer,
    child: Child,
}

impl ControlPlaneHarness {
    pub(crate) async fn spawn(prefer_push: bool) -> Result<Self> {
        ensure_control_plane_binaries_built()?;

        let runtime_registry_path = temp_path("fireline-managed-agent-runtimes");
        let peer_directory_path = temp_path("fireline-managed-agent-peers");
        let shared_state_stream_name = format!("fireline-managed-agent-suite-{}", Uuid::new_v4());
        let shared_stream_server = SharedStreamServer::spawn().await?;
        let (child, base_url) = spawn_control_plane(
            &runtime_registry_path,
            &peer_directory_path,
            &shared_stream_server.base_url,
            prefer_push,
            5_000,
            30_000,
        )
        .await?;

        Ok(Self {
            http: reqwest::Client::new(),
            base_url,
            runtime_registry_path,
            peer_directory_path,
            shared_state_stream_name,
            shared_stream_server,
            child,
        })
    }

    pub(crate) async fn create_runtime(&self, name: &str) -> Result<RuntimeDescriptor> {
        self.create_runtime_with_agent(name, &[testy_bin().display().to_string()])
            .await
    }

    pub(crate) async fn create_runtime_with_agent(
        &self,
        name: &str,
        agent_command: &[String],
    ) -> Result<RuntimeDescriptor> {
        let response = self
            .http
            .post(format!("{}/v1/runtimes", self.base_url))
            .json(&json!({
                "provider": "local",
                "host": "127.0.0.1",
                "port": 0,
                "name": name,
                "agentCommand": agent_command,
                "stateStream": self.shared_state_stream_name,
                "topology": { "components": [] }
            }))
            .send()
            .await?
            .error_for_status()?;
        let created = response.json::<RuntimeDescriptor>().await?;
        self.wait_for_status(&created.runtime_key, RuntimeStatus::Ready)
            .await
    }

    pub(crate) async fn wait_for_status(
        &self,
        runtime_key: &str,
        expected: RuntimeStatus,
    ) -> Result<RuntimeDescriptor> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let response = self
                .http
                .get(format!("{}/v1/runtimes/{runtime_key}", self.base_url))
                .send()
                .await?;
            if response.status().is_success() {
                let descriptor = response.json::<RuntimeDescriptor>().await?;
                if descriptor.status == expected {
                    return Ok(descriptor);
                }
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "timed out waiting for runtime '{runtime_key}' to become '{expected:?}'"
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub(crate) async fn issue_runtime_token(&self, runtime_key: &str) -> Result<String> {
        let response = self
            .http
            .post(format!("{}/v1/auth/runtime-token", self.base_url))
            .json(&json!({
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

    pub(crate) async fn stop_runtime(&self, runtime_key: &str) -> Result<RuntimeDescriptor> {
        self.http
            .post(format!("{}/v1/runtimes/{runtime_key}/stop", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json::<RuntimeDescriptor>()
            .await
            .map_err(anyhow::Error::from)
    }

    pub(crate) async fn shutdown(mut self) {
        shutdown_process(&mut self.child).await;
        self.shared_stream_server.shutdown().await;
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

pub(crate) fn pending_contract(case: &str, detail: &str) -> Result<()> {
    Err(anyhow!("pending managed-agent contract `{case}`: {detail}"))
}

pub(crate) async fn wait_for_stream_rows(
    state_stream_url: &str,
    required_substrings: &[&str],
    timeout: Duration,
) -> Result<String> {
    let client = DsClient::new();
    let stream = client.stream(state_stream_url);
    let deadline = tokio::time::Instant::now() + timeout;
    let mut body = String::new();

    loop {
        body.clear();

        let mut reader = stream.read().offset(Offset::Beginning).build()?;
        while let Some(chunk) = reader.next_chunk().await? {
            body.push_str(std::str::from_utf8(&chunk.data)?);
            if chunk.up_to_date {
                break;
            }
        }

        if required_substrings
            .iter()
            .all(|needle| body.contains(needle))
        {
            return Ok(body);
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for state stream rows {:?} in stream {}.\nbody:\n{}",
                required_substrings,
                state_stream_url,
                body
            ));
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub(crate) struct WebSocketTransport {
    url: String,
}

pub(crate) async fn create_session(acp_url: &str) -> Result<String> {
    sacp::Client
        .builder()
        .connect_with(
            WebSocketTransport {
                url: acp_url.to_string(),
            },
            move |cx: sacp::ConnectionTo<sacp::Agent>| async move {
                let _ = cx
                    .send_request(agent_client_protocol::InitializeRequest::new(
                        agent_client_protocol::ProtocolVersion::LATEST,
                    ))
                    .block_task()
                    .await?;

                let session = cx
                    .send_request(agent_client_protocol::NewSessionRequest::new(repo_root()))
                    .block_task()
                    .await?;

                Ok(session.session_id.to_string())
            },
        )
        .await
        .map_err(anyhow::Error::from)
}

pub(crate) async fn load_session(acp_url: &str, session_id: &str) -> Result<()> {
    let session_id = session_id.to_string();
    sacp::Client
        .builder()
        .connect_with(
            WebSocketTransport {
                url: acp_url.to_string(),
            },
            move |cx: sacp::ConnectionTo<sacp::Agent>| {
                let session_id = session_id.clone();
                async move {
                    let _ = cx
                        .send_request(agent_client_protocol::InitializeRequest::new(
                            agent_client_protocol::ProtocolVersion::LATEST,
                        ))
                        .block_task()
                        .await?;

                    let _ = cx
                        .send_request(agent_client_protocol::LoadSessionRequest::new(
                            session_id,
                            repo_root(),
                        ))
                        .block_task()
                        .await?;
                    Ok(())
                }
            },
        )
        .await
        .map_err(anyhow::Error::from)
}

pub(crate) async fn prompt_session(acp_url: &str, session_id: &str, text: &str) -> Result<()> {
    let session_id = session_id.to_string();
    let text = text.to_string();
    sacp::Client
        .builder()
        .connect_with(
            WebSocketTransport {
                url: acp_url.to_string(),
            },
            move |cx: sacp::ConnectionTo<sacp::Agent>| {
                let session_id = session_id.clone();
                let text = text.clone();
                async move {
                    let _ = cx
                        .send_request(agent_client_protocol::InitializeRequest::new(
                            agent_client_protocol::ProtocolVersion::LATEST,
                        ))
                        .block_task()
                        .await?;

                    let _response = cx
                        .send_request(agent_client_protocol::PromptRequest::new(
                            session_id,
                            vec![text.into()],
                        ))
                        .block_task()
                        .await?;
                    Ok::<(), sacp::Error>(())
                }
            },
        )
        .await
        .map_err(anyhow::Error::from)
}

impl sacp::ConnectTo<sacp::Client> for WebSocketTransport {
    async fn connect_to(
        self,
        client: impl sacp::ConnectTo<sacp::Agent>,
    ) -> Result<(), sacp::Error> {
        let (ws, _) = tokio_tungstenite::connect_async(self.url.as_str())
            .await
            .map_err(|e| sacp::util::internal_error(format!("WebSocket connect: {e}")))?;

        let (write, read) = StreamExt::split(ws);

        let outgoing = SinkExt::with(
            SinkExt::sink_map_err(write, std::io::Error::other),
            |line: String| async move {
                Ok::<_, std::io::Error>(tokio_tungstenite::tungstenite::Message::Text(line.into()))
            },
        );

        let incoming = StreamExt::filter_map(read, |msg| async move {
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

        sacp::ConnectTo::<sacp::Client>::connect_to(sacp::Lines::new(outgoing, incoming), client)
            .await
    }
}

async fn spawn_control_plane(
    runtime_registry_path: &PathBuf,
    peer_directory_path: &PathBuf,
    shared_stream_base_url: &str,
    prefer_push: bool,
    heartbeat_scan_interval_ms: u64,
    stale_timeout_ms: u64,
) -> Result<(Child, String)> {
    let listen_addr_file = temp_path("fireline-control-plane-listen-addr");
    // The control plane binds on --port 0 and writes its actual bound address
    // into this file. That closes the old TOCTOU race where the harness used
    // to reserve-and-drop a port and then hand the integer to the subprocess,
    // only to have another parallel test binary grab the same port first.
    let mut command = TokioCommand::new(control_plane_bin());
    command
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg("0")
        .arg("--listen-addr-file")
        .arg(&listen_addr_file)
        .arg("--fireline-bin")
        .arg(fireline_bin())
        .arg("--runtime-registry-path")
        .arg(runtime_registry_path)
        .arg("--peer-directory-path")
        .arg(peer_directory_path)
        .arg("--startup-timeout-ms")
        .arg("20000")
        .arg("--stop-timeout-ms")
        .arg("10000")
        .arg("--heartbeat-scan-interval-ms")
        .arg(heartbeat_scan_interval_ms.to_string())
        .arg("--stale-timeout-ms")
        .arg(stale_timeout_ms.to_string())
        .arg("--shared-stream-base-url")
        .arg(shared_stream_base_url)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if prefer_push {
        command.arg("--prefer-push");
    }

    let mut child = command.spawn().context("spawn fireline-control-plane")?;
    let bound_addr = wait_for_listen_addr_file(&listen_addr_file, &mut child).await?;
    let base_url = format!("http://{bound_addr}");
    wait_for_http_ok(&format!("{base_url}/healthz"), &mut child).await?;
    let _ = std::fs::remove_file(&listen_addr_file);
    Ok((child, base_url))
}

async fn wait_for_listen_addr_file(path: &PathBuf, child: &mut Child) -> Result<String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(anyhow!(
                "control plane exited before reporting its bound address: {status}"
            ));
        }

        match std::fs::read_to_string(path) {
            Ok(contents) => {
                let trimmed = contents.trim();
                if !trimmed.is_empty() {
                    return Ok(trimmed.to_string());
                }
            }
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                return Err(anyhow::Error::from(err).context(format!(
                    "read control-plane listen-addr file {}",
                    path.display()
                )));
            }
            Err(_) => {}
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for control plane listen-addr file at {}",
                path.display()
            ));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
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
            Ok(_) | Err(_) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
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

// =============================================================================
// Configurable harness spec
// =============================================================================
//
// The original `LocalRuntimeHarness::spawn(name)` and
// `ControlPlaneHarness::create_runtime(name)` entry points hardcoded default
// topology, empty resources, and embedded-vs-shared stream choice. That
// rigidity was blocking most of the pending per-primitive contract tests
// because they need resources in the launch spec, or specific topology
// components (approval gate, fs backend), or a shared-stream mode for
// cross-runtime reads.
//
// `ManagedAgentHarnessSpec` is a single builder-style spec that both harnesses
// consume via `spawn_with()` / `create_runtime_from_spec()`. The existing
// methods stay as thin wrappers over default specs for backwards compat.

/// Controls whether the runtime writes to its own embedded durable-streams
/// server or to a shared external one. Shared mode is required for tests
/// that need the stream to outlive the runtime process (e.g. cold-start
/// resume, cross-runtime virtual-fs reads).
#[derive(Clone, Debug)]
pub(crate) enum StreamMode {
    /// Embedded durable-streams server inside the runtime process. Dies
    /// with the runtime. This is the default for quick local tests.
    Embedded,
    /// External shared stream. The URL is the shared durable-streams
    /// server's base URL; the stream name is the logical stream the runtime
    /// appends to. Both produced by `SharedStreamServer::spawn` or
    /// `ControlPlaneHarness::spawn`.
    SharedExternal { base_url: String, stream_name: String },
}

/// Builder-style spec for provisioning a managed-agent test runtime. Both
/// `LocalRuntimeHarness::spawn_with` and
/// `ControlPlaneHarness::create_runtime_from_spec` consume this.
#[derive(Clone)]
pub(crate) struct ManagedAgentHarnessSpec {
    pub name: String,
    pub agent_command: Vec<String>,
    pub topology: TopologySpec,
    pub resources: Vec<ResourceRef>,
    pub stream_mode: StreamMode,
    pub prefer_push: bool,
}

impl ManagedAgentHarnessSpec {
    /// Fresh default spec: testy agent, default topology, no resources,
    /// embedded stream. Every field is then mutable via the `with_*`
    /// builder methods.
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            agent_command: vec![testy_bin().display().to_string()],
            topology: TopologySpec::default(),
            resources: Vec::new(),
            stream_mode: StreamMode::Embedded,
            prefer_push: false,
        }
    }

    pub(crate) fn with_agent_command(mut self, cmd: Vec<String>) -> Self {
        self.agent_command = cmd;
        self
    }

    /// Swap the agent binary for `fireline-testy-load`, which implements
    /// ACP `session/load` (same-process reattach only — does NOT survive
    /// process restart).
    pub(crate) fn with_testy_load_agent(mut self) -> Self {
        self.agent_command = vec![testy_load_bin().display().to_string()];
        self
    }

    pub(crate) fn with_topology(mut self, topology: TopologySpec) -> Self {
        self.topology = topology;
        self
    }

    pub(crate) fn with_resources(mut self, resources: Vec<ResourceRef>) -> Self {
        self.resources = resources;
        self
    }

    pub(crate) fn with_shared_stream(
        mut self,
        base_url: impl Into<String>,
        stream_name: impl Into<String>,
    ) -> Self {
        self.stream_mode = StreamMode::SharedExternal {
            base_url: base_url.into(),
            stream_name: stream_name.into(),
        };
        self
    }

    pub(crate) fn with_prefer_push(mut self, prefer_push: bool) -> Self {
        self.prefer_push = prefer_push;
        self
    }
}

impl LocalRuntimeHarness {
    /// Spec-based alternative to `spawn(name)`. Supports configurable agent
    /// command, topology, resources, and stream mode. Resources are mounted
    /// via `LocalPathMounter` before the runtime boots so they are visible
    /// to the conductor.
    pub(crate) async fn spawn_with(spec: ManagedAgentHarnessSpec) -> Result<Self> {
        let (state_stream, external_state_stream_url) = match &spec.stream_mode {
            StreamMode::Embedded => (None, None),
            StreamMode::SharedExternal { base_url, stream_name } => (
                Some(stream_name.clone()),
                Some(format!("{}/{}", base_url.trim_end_matches('/'), stream_name)),
            ),
        };

        // Pre-mount LocalPath resources so the runtime sees them as real
        // directories. Other mounter kinds (git, s3) are accepted by the
        // spec but not yet implemented — they would need their own async
        // mount step here.
        let mounter = LocalPathMounter::new();
        let runtime_key = format!("runtime:{}", Uuid::new_v4());
        let mut mounted_resources = Vec::new();
        for resource in &spec.resources {
            if let Some(mounted) = mounter
                .mount(resource, &runtime_key)
                .await
                .context("pre-mount resource before spawn")?
            {
                mounted_resources.push(mounted);
            }
        }

        let handle = start(BootstrapConfig {
            host: "127.0.0.1".parse::<IpAddr>()?,
            port: 0,
            name: spec.name.clone(),
            runtime_key,
            node_id: "node:managed-agent-suite".to_string(),
            agent_command: spec.agent_command,
            mounted_resources,
            state_stream,
            stream_storage: None,
            peer_directory_path: temp_path("fireline-managed-agent-peers"),
            control_plane_url: None,
            external_state_stream_url,
            topology: spec.topology,
        })
        .await?;

        Ok(Self { handle })
    }
}

impl ControlPlaneHarness {
    /// Spec-based alternative to `create_runtime_with_agent`. Passes the
    /// spec's topology and resources through to the control plane
    /// `POST /v1/runtimes` request body, unlike the legacy method which
    /// always sent empty topology and no resources.
    pub(crate) async fn create_runtime_from_spec(
        &self,
        spec: ManagedAgentHarnessSpec,
    ) -> Result<RuntimeDescriptor> {
        // If the spec requests SharedExternal, use the requested stream name;
        // otherwise fall back to the harness's default shared stream name
        // (Embedded isn't meaningful in control-plane mode since the control
        // plane always writes to the shared stream).
        let stream_name = match &spec.stream_mode {
            StreamMode::SharedExternal { stream_name, .. } => stream_name.clone(),
            StreamMode::Embedded => self.shared_state_stream_name.clone(),
        };

        let body = json!({
            "provider": "local",
            "host": "127.0.0.1",
            "port": 0,
            "name": spec.name,
            "agentCommand": spec.agent_command,
            "stateStream": stream_name,
            "topology": spec.topology,
            "resources": spec.resources,
        });

        let response = self
            .http
            .post(format!("{}/v1/runtimes", self.base_url))
            .json(&body)
            .send()
            .await
            .context("POST /v1/runtimes with spec")?
            .error_for_status()
            .context("control plane rejected spec-based create")?;
        let created = response.json::<RuntimeDescriptor>().await?;
        self.wait_for_status(&created.runtime_key, RuntimeStatus::Ready)
            .await
    }
}

// =============================================================================
// Parsed stream oracle
// =============================================================================
//
// The original `wait_for_stream_rows` helper polls for substring presence and
// returns as soon as every required needle appears ONCE. That's adequate for a
// smoke check but wrong for count-based assertions: a test that wants to see
// three `prompt_turn` events will call wait_for_stream_rows("prompt_turn"),
// receive the body after the first prompt_turn appears, then count only one
// and fail non-deterministically.
//
// The helpers below parse the stream body into structured envelopes and
// support count-aware polling and predicate-based waiting.

/// A parsed durable-streams state envelope — one line in the stream body is
/// one of these. Wraps a raw `serde_json::Value` for flexibility.
#[derive(Debug, Clone)]
pub(crate) struct StateEnvelope {
    pub raw: JsonValue,
}

impl StateEnvelope {
    pub(crate) fn envelope_type(&self) -> Option<&str> {
        self.raw.get("type").and_then(|v| v.as_str())
    }

    pub(crate) fn key(&self) -> Option<&str> {
        self.raw.get("key").and_then(|v| v.as_str())
    }

    pub(crate) fn value(&self) -> Option<&JsonValue> {
        self.raw.get("value")
    }

    pub(crate) fn operation(&self) -> Option<&str> {
        self.raw
            .get("headers")
            .and_then(|h| h.get("operation"))
            .and_then(|v| v.as_str())
    }
}

fn parse_envelopes(body: &str) -> Result<Vec<StateEnvelope>> {
    // durable-streams returns JSON envelopes separated either by newlines or
    // as a single JSON array. Handle both shapes.
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    if trimmed.starts_with('[') {
        let arr: Vec<JsonValue> = serde_json::from_str(trimmed)
            .with_context(|| format!("parse stream body as JSON array: {trimmed}"))?;
        return Ok(arr.into_iter().map(|raw| StateEnvelope { raw }).collect());
    }

    trimmed
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let raw: JsonValue = serde_json::from_str(line)
                .with_context(|| format!("parse stream envelope line: {line}"))?;
            Ok(StateEnvelope { raw })
        })
        .collect()
}

/// Read every envelope currently on the stream from offset 0 to the live
/// edge. Returns a parsed `Vec<StateEnvelope>` — callers that need
/// count/order assertions should use this instead of `wait_for_state_rows`.
pub(crate) async fn read_all_events(state_stream_url: &str) -> Result<Vec<StateEnvelope>> {
    let client = DsClient::new();
    let stream = client.stream(state_stream_url);
    let mut reader = stream
        .read()
        .offset(Offset::Beginning)
        .build()
        .context("build stream reader from Offset::Beginning")?;

    let mut body = String::new();
    while let Some(chunk) = reader
        .next_chunk()
        .await
        .context("read stream chunk for parsed events")?
    {
        body.push_str(
            std::str::from_utf8(&chunk.data)
                .context("stream chunk is not valid UTF-8")?,
        );
        if chunk.up_to_date {
            break;
        }
    }

    parse_envelopes(&body)
}

/// Count the envelopes currently on the stream whose `type` field equals
/// `type_name`. Does a single read, not a poll.
pub(crate) async fn count_events(state_stream_url: &str, type_name: &str) -> Result<usize> {
    let events = read_all_events(state_stream_url).await?;
    Ok(events
        .iter()
        .filter(|env| env.envelope_type() == Some(type_name))
        .count())
}

/// Poll the stream until at least `target_count` events of the given type
/// are visible, or the timeout fires. Returns the matching envelopes on
/// success. Use this instead of `wait_for_state_rows` for count-based
/// assertions — it fixes the "returns too early" race in the substring
/// helper.
pub(crate) async fn wait_for_event_count(
    state_stream_url: &str,
    type_name: &str,
    target_count: usize,
    timeout: Duration,
) -> Result<Vec<StateEnvelope>> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let events = read_all_events(state_stream_url).await?;
        let matches: Vec<StateEnvelope> = events
            .into_iter()
            .filter(|env| env.envelope_type() == Some(type_name))
            .collect();
        if matches.len() >= target_count {
            return Ok(matches);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for {target_count} '{type_name}' events in stream {state_stream_url}; saw {}",
                matches.len()
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Poll the stream until at least one envelope matches `pred`, or the
/// timeout fires. Returns the first matching envelope.
pub(crate) async fn wait_for_event<F>(
    state_stream_url: &str,
    mut pred: F,
    timeout: Duration,
) -> Result<StateEnvelope>
where
    F: FnMut(&StateEnvelope) -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let events = read_all_events(state_stream_url).await?;
        if let Some(found) = events.into_iter().find(|env| pred(env)) {
            return Ok(found);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for matching event in stream {state_stream_url}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// =============================================================================
// Typed state entity decoding
// =============================================================================
//
// Substring matching is adequate for smoke checks but wrong for semantic
// assertions. `StateEnvelope::decode()` tries to parse the envelope into one
// of the known entity types from production code:
//
// - `SessionRecord` — from `fireline_conductor::session`
// - `PersistedRuntimeSpec` — from `fireline_conductor::runtime`
// - `FsOpRecord` — from `fireline_components::fs_backend`
// - `RuntimeStreamFileRecord` — same
//
// Tests get typed access to every load-bearing state entity without
// reimplementing the projector, which is exactly what the second-round
// review asked for ("stop making tests scrape strings").

/// Decoded durable state entity. One of the known typed variants from
/// production code, or `Unknown` if the entity type isn't recognized (or
/// isn't yet typed in this module).
#[derive(Debug, Clone)]
pub(crate) enum DecodedStateEntity {
    Session(SessionRecord),
    RuntimeSpec(PersistedRuntimeSpec),
    FsOp(FsOpRecord),
    RuntimeStreamFile(RuntimeStreamFileRecord),
    /// Entity type was recognized as a string but no typed decoder exists,
    /// or the envelope was an operation (delete/insert) without a full
    /// value. Holds the raw value for custom decoding in the test.
    Unknown {
        entity_type: String,
        value: Option<JsonValue>,
    },
}

impl StateEnvelope {
    /// Attempt to decode this envelope into a typed `DecodedStateEntity`.
    /// Returns `None` if the envelope has no recognizable `type` field.
    pub(crate) fn decode(&self) -> Option<DecodedStateEntity> {
        let entity_type = self.envelope_type()?;
        let value = self.value().cloned();

        match entity_type {
            "session" => value
                .as_ref()
                .and_then(|v| serde_json::from_value::<SessionRecord>(v.clone()).ok())
                .map(DecodedStateEntity::Session)
                .or_else(|| {
                    Some(DecodedStateEntity::Unknown {
                        entity_type: entity_type.to_string(),
                        value,
                    })
                }),
            "runtime_spec" => value
                .as_ref()
                .and_then(|v| serde_json::from_value::<PersistedRuntimeSpec>(v.clone()).ok())
                .map(DecodedStateEntity::RuntimeSpec)
                .or_else(|| {
                    Some(DecodedStateEntity::Unknown {
                        entity_type: entity_type.to_string(),
                        value,
                    })
                }),
            "fs_op" => value
                .as_ref()
                .and_then(|v| serde_json::from_value::<FsOpRecord>(v.clone()).ok())
                .map(DecodedStateEntity::FsOp)
                .or_else(|| {
                    Some(DecodedStateEntity::Unknown {
                        entity_type: entity_type.to_string(),
                        value,
                    })
                }),
            "runtime_stream_file" => value
                .as_ref()
                .and_then(|v| serde_json::from_value::<RuntimeStreamFileRecord>(v.clone()).ok())
                .map(DecodedStateEntity::RuntimeStreamFile)
                .or_else(|| {
                    Some(DecodedStateEntity::Unknown {
                        entity_type: entity_type.to_string(),
                        value,
                    })
                }),
            other => Some(DecodedStateEntity::Unknown {
                entity_type: other.to_string(),
                value,
            }),
        }
    }

    /// Convenience: decode into `SessionRecord` if this envelope is a
    /// session entity with a full value. Returns `None` otherwise.
    pub(crate) fn as_session_record(&self) -> Option<SessionRecord> {
        match self.decode()? {
            DecodedStateEntity::Session(record) => Some(record),
            _ => None,
        }
    }

    /// Convenience: decode into `FsOpRecord`.
    pub(crate) fn as_fs_op(&self) -> Option<FsOpRecord> {
        match self.decode()? {
            DecodedStateEntity::FsOp(record) => Some(record),
            _ => None,
        }
    }

    /// Convenience: decode into `RuntimeStreamFileRecord`.
    pub(crate) fn as_runtime_stream_file(&self) -> Option<RuntimeStreamFileRecord> {
        match self.decode()? {
            DecodedStateEntity::RuntimeStreamFile(record) => Some(record),
            _ => None,
        }
    }

    /// Convenience: decode into `PersistedRuntimeSpec`.
    pub(crate) fn as_runtime_spec(&self) -> Option<PersistedRuntimeSpec> {
        match self.decode()? {
            DecodedStateEntity::RuntimeSpec(record) => Some(record),
            _ => None,
        }
    }
}

/// Read every envelope currently on the stream and return the typed
/// session records among them.
pub(crate) async fn read_session_records(state_stream_url: &str) -> Result<Vec<SessionRecord>> {
    let envelopes = read_all_events(state_stream_url).await?;
    Ok(envelopes
        .iter()
        .filter_map(StateEnvelope::as_session_record)
        .collect())
}

/// Read every envelope currently on the stream and return the typed
/// fs_op records for the given session id.
pub(crate) async fn read_fs_ops_for_session(
    state_stream_url: &str,
    session_id: &str,
) -> Result<Vec<FsOpRecord>> {
    let envelopes = read_all_events(state_stream_url).await?;
    Ok(envelopes
        .iter()
        .filter_map(StateEnvelope::as_fs_op)
        .filter(|record| record.session_id == session_id)
        .collect())
}

/// Read every envelope and return the typed persisted runtime specs.
/// There is usually exactly one per runtime_key.
pub(crate) async fn read_persisted_runtime_specs(
    state_stream_url: &str,
) -> Result<Vec<PersistedRuntimeSpec>> {
    let envelopes = read_all_events(state_stream_url).await?;
    Ok(envelopes
        .iter()
        .filter_map(StateEnvelope::as_runtime_spec)
        .collect())
}

/// Poll until a session record for the given session id appears on the
/// stream, or the timeout fires. Returns the decoded record.
pub(crate) async fn wait_for_session_record(
    state_stream_url: &str,
    session_id: &str,
    timeout: Duration,
) -> Result<SessionRecord> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let records = read_session_records(state_stream_url).await?;
        if let Some(found) = records.into_iter().find(|r| r.session_id == session_id) {
            return Ok(found);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for session record '{session_id}' in stream {state_stream_url}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Append a synthetic `approval_resolved` permission event to a state
/// stream as an "external producer". Used by approval-gate contract
/// tests that need to unblock a suspended prompt by simulating an
/// approval service write.
pub(crate) async fn append_approval_resolved(
    state_stream_url: &str,
    session_id: &str,
    request_id: &str,
    allow: bool,
) -> Result<()> {
    let client = DsClient::new();
    let mut stream = client.stream(state_stream_url);
    stream.set_content_type("application/json");
    let producer = stream
        .producer(format!("test-approval-writer-{}", Uuid::new_v4()))
        .content_type("application/json")
        .build();

    let created_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let envelope = serde_json::json!({
        "type": "permission",
        "key": format!("{session_id}:{request_id}:resolved"),
        "headers": { "operation": "insert" },
        "value": {
            "kind": "approval_resolved",
            "sessionId": session_id,
            "requestId": request_id,
            "allow": allow,
            "resolvedBy": "test-approval-writer",
            "createdAtMs": created_at_ms,
        }
    });

    producer.append_json(&envelope);
    producer
        .flush()
        .await
        .context("flush external approval writer")?;
    Ok(())
}

/// Wait for the approval gate to publish its `permission_request`
/// envelope for a given session and return `(session_id, request_id)`
/// parsed from the envelope. The request_id is reconstructed from the
/// envelope key, which has the shape `"<session>:<request_id>"` as
/// emitted by `ApprovalGateComponent::emit_permission_request`.
pub(crate) async fn wait_for_permission_request(
    state_stream_url: &str,
    session_id: &str,
    timeout: Duration,
) -> Result<String> {
    let envelope = wait_for_event(
        state_stream_url,
        |env| {
            if env.envelope_type() != Some("permission") {
                return false;
            }
            let Some(value) = env.value() else {
                return false;
            };
            value
                .get("kind")
                .and_then(|v| v.as_str())
                .map(|k| k == "permission_request")
                .unwrap_or(false)
                && value
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .map(|s| s == session_id)
                    .unwrap_or(false)
        },
        timeout,
    )
    .await?;

    envelope
        .value()
        .and_then(|value| value.get("requestId"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow!(
                "permission_request envelope missing requestId field for session '{session_id}'"
            )
        })
}
