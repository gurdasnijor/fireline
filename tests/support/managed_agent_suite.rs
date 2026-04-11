#![allow(dead_code)]

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::{Command as StdCommand, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use axum::Router;
use durable_streams::{Client as DsClient, Offset};
use fireline::bootstrap::{BootstrapConfig, BootstrapHandle, start};
use fireline::runtime_host::{RuntimeDescriptor, RuntimeStatus};
use fireline_conductor::topology::TopologySpec;
use futures::{SinkExt, StreamExt};
use serde_json::json;
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
        let base_url = format!("http://127.0.0.1:{}", reserve_port()?);
        let shared_stream_server = SharedStreamServer::spawn().await?;
        let child = spawn_control_plane(
            &base_url,
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
    base_url: &str,
    runtime_registry_path: &PathBuf,
    peer_directory_path: &PathBuf,
    shared_stream_base_url: &str,
    prefer_push: bool,
    heartbeat_scan_interval_ms: u64,
    stale_timeout_ms: u64,
) -> Result<Child> {
    let port = base_url
        .rsplit(':')
        .next()
        .ok_or_else(|| anyhow!("missing control-plane port"))?;
    let mut command = TokioCommand::new(control_plane_bin());
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
    wait_for_http_ok(&format!("{base_url}/healthz"), &mut child).await?;
    Ok(child)
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

fn reserve_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .context("bind ephemeral port")?;
    Ok(listener.local_addr()?.port())
}
