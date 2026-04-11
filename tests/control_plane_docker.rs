use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use agent_client_protocol_test::testy::TestyCommand;
use anyhow::{Context, Result, anyhow};
use axum::Router;
use durable_streams::{Client as DsClient, Offset};
use fireline::runtime_host::{RuntimeDescriptor, RuntimeProviderKind, RuntimeStatus};
use futures::{SinkExt, StreamExt};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;
use uuid::Uuid;

struct WebSocketTransport {
    url: String,
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

#[tokio::test]
async fn control_plane_supports_local_and_docker_runtimes_against_one_shared_state_plane()
-> Result<()> {
    if !docker_available()? {
        eprintln!("skipping docker control-plane integration test: docker daemon unavailable");
        return Ok(());
    }

    ensure_control_plane_binaries_built()?;
    let shared_ds = SharedStreamServer::spawn().await?;
    let runtime_registry_path = temp_path("fireline-control-plane-runtimes");
    let peer_directory_path = temp_path("fireline-control-plane-peers");
    let base_url = format!("http://127.0.0.1:{}", reserve_port()?);
    let docker_image = format!("fireline-runtime:test-{}", Uuid::new_v4());
    let mut control_plane = spawn_control_plane(
        &base_url,
        &runtime_registry_path,
        &peer_directory_path,
        &shared_ds.base_url,
        &docker_image,
    )
    .await?;
    let mut created_runtime_keys = Vec::new();

    let result = async {
        let client = reqwest::Client::new();
        let local = create_runtime(
            &client,
            &base_url,
            RuntimeProviderKind::Local,
            "agent-local",
            vec![testy_bin()],
        )
        .await?;
        created_runtime_keys.push(local.runtime_key.clone());

        let mut docker_runtimes = Vec::new();
        for index in 0..4 {
            let runtime = create_runtime(
                &client,
                &base_url,
                RuntimeProviderKind::Docker,
                &format!("agent-docker-{}", index + 1),
                vec!["/usr/local/bin/fireline-testy".to_string()],
            )
            .await?;
            created_runtime_keys.push(runtime.runtime_key.clone());
            docker_runtimes.push(runtime);
        }

        let local_ready =
            wait_for_status(&base_url, &local.runtime_key, RuntimeStatus::Ready).await?;
        let mut docker_ready = Vec::new();
        for runtime in &docker_runtimes {
            docker_ready.push(
                wait_for_status(&base_url, &runtime.runtime_key, RuntimeStatus::Ready).await?,
            );
        }

        let listed = client
            .get(format!("{base_url}/v1/runtimes"))
            .send()
            .await?
            .error_for_status()?
            .json::<Vec<RuntimeDescriptor>>()
            .await?;
        assert_eq!(listed.len(), 5);
        assert_eq!(
            listed
                .iter()
                .filter(|runtime| runtime.provider == RuntimeProviderKind::Local)
                .count(),
            1
        );
        assert_eq!(
            listed
                .iter()
                .filter(|runtime| runtime.provider == RuntimeProviderKind::Docker)
                .count(),
            4
        );

        for runtime in std::iter::once(&local_ready).chain(docker_ready.iter()) {
            assert!(
                runtime
                    .state
                    .url
                    .starts_with(&format!("{}/", shared_ds.base_url)),
                "runtime should advertise the shared durable-streams url: {:?}",
                runtime.state
            );
        }

        let token_for_local =
            issue_runtime_token(&client, &base_url, &local_ready.runtime_key).await?;
        let cross_runtime_heartbeat = client
            .post(format!(
                "{base_url}/v1/runtimes/{}/heartbeat",
                docker_ready[0].runtime_key
            ))
            .bearer_auth(&token_for_local)
            .json(&json!({
                "tsMs": now_ms(),
                "metrics": null
            }))
            .send()
            .await?;
        assert!(matches!(
            cross_runtime_heartbeat.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));

        let peers = yopo::prompt(
            WebSocketTransport {
                url: local_ready.acp.url.clone(),
            },
            TestyCommand::CallTool {
                server: "fireline-peer".to_string(),
                tool: "list_peers".to_string(),
                params: json!({}),
            }
            .to_prompt(),
        )
        .await?;
        assert!(peers.contains("agent-local"));
        assert!(peers.contains("agent-docker-1"));

        let prompt_peer = yopo::prompt(
            WebSocketTransport {
                url: local_ready.acp.url.clone(),
            },
            TestyCommand::CallTool {
                server: "fireline-peer".to_string(),
                tool: "prompt_peer".to_string(),
                params: json!({
                    "agentName": "agent-docker-1",
                    "prompt": TestyCommand::Echo {
                        message: "hello across docker mesh".to_string(),
                    }
                    .to_prompt(),
                }),
            }
            .to_prompt(),
        )
        .await?;
        assert!(prompt_peer.contains("agent-docker-1"));
        assert!(prompt_peer.contains("hello across docker mesh"));

        for runtime in std::iter::once(&local_ready).chain(docker_ready.iter()) {
            let body = wait_for_state_contains(
                &runtime.state.url,
                "\"type\":\"runtime_instance\"",
                Duration::from_secs(10),
            )
            .await?;
            assert!(
                body.contains("\"type\":\"runtime_instance\""),
                "shared stream should contain runtime startup state: {}",
                runtime.state.url
            );
        }

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let local_body = read_state_stream(&local_ready.state.url).await?;
            let docker_body = read_state_stream(&docker_ready[0].state.url).await?;

            let parent = find_prompt_turn(&local_body, |text| {
                text.contains("\"tool\":\"prompt_peer\"")
            });
            let child = find_prompt_turn(&docker_body, |text| {
                text.contains("hello across docker mesh")
            });
            let edge = find_child_session_edge(&local_body);

            if let (Some(parent), Some(child), Some(edge)) = (parent, child, edge) {
                assert_eq!(
                    child.parent_prompt_turn_id.as_deref(),
                    Some(parent.prompt_turn_id.as_str())
                );
                assert_eq!(child.trace_id.as_deref(), parent.trace_id.as_deref());
                assert_eq!(edge.parent_runtime_id, local_ready.runtime_id);
                assert_eq!(edge.parent_prompt_turn_id, parent.prompt_turn_id);
                assert_eq!(edge.child_runtime_id, docker_ready[0].runtime_id);
                assert_eq!(edge.parent_session_id, parent.session_id);
                assert_eq!(edge.child_session_id, child.session_id);
                assert_eq!(edge.trace_id.as_deref(), parent.trace_id.as_deref());
                break;
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "timed out waiting for cross-runtime lineage in shared streams"
                ));
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let stopped = client
            .post(format!(
                "{base_url}/v1/runtimes/{}/stop",
                docker_ready[0].runtime_key
            ))
            .send()
            .await?
            .error_for_status()?
            .json::<RuntimeDescriptor>()
            .await?;
        assert_eq!(stopped.status, RuntimeStatus::Stopped);

        Ok(())
    }
    .await;

    cleanup_runtimes(&base_url, &created_runtime_keys).await;
    shutdown_process(&mut control_plane).await;
    shared_ds.shutdown().await;
    result
}

async fn create_runtime(
    client: &reqwest::Client,
    base_url: &str,
    provider: RuntimeProviderKind,
    name: &str,
    agent_command: Vec<String>,
) -> Result<RuntimeDescriptor> {
    client
        .post(format!("{base_url}/v1/runtimes"))
        .json(&json!({
            "provider": match provider {
                RuntimeProviderKind::Local => "local",
                RuntimeProviderKind::Docker => "docker",
            },
            "host": "127.0.0.1",
            "port": 0,
            "name": name,
            "agentCommand": agent_command,
            "topology": { "components": [] }
        }))
        .send()
        .await?
        .error_for_status()?
        .json::<RuntimeDescriptor>()
        .await
        .context("decode create runtime response")
}

async fn wait_for_status(
    base_url: &str,
    runtime_key: &str,
    expected: RuntimeStatus,
) -> Result<RuntimeDescriptor> {
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let response = client
            .get(format!("{base_url}/v1/runtimes/{runtime_key}"))
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
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn issue_runtime_token(
    client: &reqwest::Client,
    base_url: &str,
    runtime_key: &str,
) -> Result<String> {
    let response = client
        .post(format!("{base_url}/v1/auth/runtime-token"))
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

async fn read_state_stream(state_stream_url: &str) -> Result<String> {
    let client = DsClient::new();
    let stream = client.stream(state_stream_url);
    let mut reader = stream.read().offset(Offset::Beginning).build()?;
    let mut body = String::new();
    while let Some(chunk) = reader.next_chunk().await? {
        body.push_str(std::str::from_utf8(&chunk.data)?);
        if chunk.up_to_date {
            break;
        }
    }
    Ok(body)
}

async fn wait_for_state_contains(
    state_stream_url: &str,
    needle: &str,
    timeout: Duration,
) -> Result<String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let body = read_state_stream(state_stream_url).await?;
        if body.contains(needle) {
            return Ok(body);
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(body);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
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

fn fireline_bin() -> PathBuf {
    target_bin("fireline")
}

fn control_plane_bin() -> PathBuf {
    target_bin("fireline-control-plane")
}

fn testy_bin() -> String {
    target_bin("fireline-testy").display().to_string()
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

async fn spawn_control_plane(
    base_url: &str,
    runtime_registry_path: &PathBuf,
    peer_directory_path: &PathBuf,
    shared_stream_base_url: &str,
    docker_image: &str,
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
        .arg("--provider")
        .arg("docker")
        .arg("--prefer-push")
        .arg("--shared-stream-base-url")
        .arg(shared_stream_base_url)
        .arg("--docker-build-context")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
        .arg("--docker-image")
        .arg(docker_image)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")));

    let mut child = command.spawn().context("spawn fireline-control-plane")?;
    wait_for_http_ok(&format!("{base_url}/healthz"), &mut child).await?;
    Ok(child)
}

async fn wait_for_http_ok(url: &str, child: &mut Child) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
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
                tokio::time::sleep(Duration::from_millis(100)).await;
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

async fn cleanup_runtimes(base_url: &str, runtime_keys: &[String]) {
    let client = reqwest::Client::new();
    for runtime_key in runtime_keys {
        let _ = client
            .delete(format!("{base_url}/v1/runtimes/{runtime_key}"))
            .send()
            .await;
    }
}

fn docker_available() -> Result<bool> {
    let status = std::process::Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("check docker availability")?;
    Ok(status.success())
}

fn temp_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()))
}

fn reserve_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .context("bind ephemeral port")?;
    Ok(listener.local_addr()?.port())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}

#[derive(Debug)]
struct PromptTurnEvent {
    prompt_turn_id: String,
    session_id: String,
    trace_id: Option<String>,
    parent_prompt_turn_id: Option<String>,
}

#[derive(Debug)]
struct ChildSessionEdgeEvent {
    parent_runtime_id: String,
    parent_session_id: String,
    parent_prompt_turn_id: String,
    child_runtime_id: String,
    child_session_id: String,
    trace_id: Option<String>,
}

fn find_prompt_turn(body: &str, predicate: impl Fn(&str) -> bool) -> Option<PromptTurnEvent> {
    parse_state_events(body).into_iter().find_map(|event| {
        if event.get("type")?.as_str()? != "prompt_turn" {
            return None;
        }

        let value = event.get("value")?;
        let text = value.get("text").and_then(Value::as_str).unwrap_or("");
        if !predicate(text) {
            return None;
        }

        Some(PromptTurnEvent {
            prompt_turn_id: value.get("promptTurnId")?.as_str()?.to_string(),
            session_id: value.get("sessionId")?.as_str()?.to_string(),
            trace_id: value
                .get("traceId")
                .and_then(Value::as_str)
                .map(str::to_string),
            parent_prompt_turn_id: value
                .get("parentPromptTurnId")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    })
}

fn find_child_session_edge(body: &str) -> Option<ChildSessionEdgeEvent> {
    parse_state_events(body).into_iter().find_map(|event| {
        if event.get("type")?.as_str()? != "child_session_edge" {
            return None;
        }

        let value = event.get("value")?;
        Some(ChildSessionEdgeEvent {
            parent_runtime_id: value.get("parentRuntimeId")?.as_str()?.to_string(),
            parent_session_id: value.get("parentSessionId")?.as_str()?.to_string(),
            parent_prompt_turn_id: value.get("parentPromptTurnId")?.as_str()?.to_string(),
            child_runtime_id: value.get("childRuntimeId")?.as_str()?.to_string(),
            child_session_id: value.get("childSessionId")?.as_str()?.to_string(),
            trace_id: value
                .get("traceId")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    })
}

fn parse_state_events(body: &str) -> Vec<Value> {
    match serde_json::from_str::<Value>(body) {
        Ok(Value::Array(events)) => events,
        Ok(value) => vec![value],
        Err(_) => {
            let mut stream = serde_json::Deserializer::from_str(body).into_iter::<Value>();
            std::iter::from_fn(move || stream.next())
                .filter_map(|result| result.ok())
                .collect()
        }
    }
}
