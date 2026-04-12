use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use fireline_runtime::RuntimeRegistry;
use fireline_runtime::runtime_host::{
    Endpoint, RuntimeDescriptor, RuntimeProviderKind, RuntimeStatus,
};
use reqwest::StatusCode;
use serde_json::json;
use tokio::process::{Child, Command};
use uuid::Uuid;

#[path = "support/stream_server.rs"]
mod stream_server;

fn fireline_bin() -> PathBuf {
    target_bin("fireline")
}

fn control_plane_bin() -> PathBuf {
    target_bin("fireline-control-plane")
}

fn testy_bin() -> PathBuf {
    target_bin("fireline-testy")
}

fn temp_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()))
}

#[tokio::test]
async fn prefer_push_runtime_publishes_final_descriptor_through_control_plane() -> Result<()> {
    ensure_control_plane_binaries_built()?;
    assert_runtime_lifecycle_round_trip(true).await
}

#[tokio::test]
async fn polling_runtime_publishes_final_descriptor_through_control_plane() -> Result<()> {
    ensure_control_plane_binaries_built()?;
    assert_runtime_lifecycle_round_trip(false).await
}

async fn assert_runtime_lifecycle_round_trip(prefer_push: bool) -> Result<()> {
    let runtime_registry_path = temp_path("fireline-control-plane-runtimes");
    let peer_directory_path = temp_path("fireline-control-plane-peers");
    let shared_stream_server = stream_server::TestStreamServer::spawn().await?;
    let base_url = format!("http://127.0.0.1:{}", reserve_port()?);
    let mut control_plane = spawn_control_plane(
        &base_url,
        &runtime_registry_path,
        &peer_directory_path,
        prefer_push,
        5_000,
        30_000,
    )
    .await?;

    let result = async {
        let client = reqwest::Client::new();
        let response = client
            .post(format!("{base_url}/v1/runtimes"))
            .json(&json!({
                "provider": "local",
                "host": "127.0.0.1",
                "port": 0,
                "name": "push-test",
                "agentCommand": [testy_bin()],
                "durableStreamsUrl": shared_stream_server.base_url.clone(),
                "topology": { "components": [] }
            }))
            .send()
            .await?
            .error_for_status()?;
        let created = response.json::<RuntimeDescriptor>().await?;
        let runtime =
            wait_for_status(&base_url, &created.runtime_key, RuntimeStatus::Ready).await?;

        assert_eq!(runtime.status, RuntimeStatus::Ready);
        assert_eq!(runtime.provider, RuntimeProviderKind::Local);
        assert!(runtime.runtime_id.starts_with("fireline:push-test:"));
        assert_eq!(runtime.provider_instance_id, runtime.runtime_id);
        assert!(runtime.acp.url.starts_with("ws://"));
        assert!(runtime.state.url.starts_with("http://"));

        let stopped = client
            .post(format!(
                "{base_url}/v1/runtimes/{}/stop",
                runtime.runtime_key
            ))
            .send()
            .await?
            .error_for_status()?
            .json::<RuntimeDescriptor>()
            .await?;
        assert_eq!(stopped.status, RuntimeStatus::Stopped);

        let deleted = client
            .delete(format!("{base_url}/v1/runtimes/{}", runtime.runtime_key))
            .send()
            .await?
            .error_for_status()?
            .json::<RuntimeDescriptor>()
            .await?;
        assert_eq!(deleted.runtime_key, runtime.runtime_key);
        Ok(())
    }
    .await;

    shutdown_process(&mut control_plane).await;
    shared_stream_server.shutdown().await;
    result
}

#[tokio::test]
async fn register_becomes_stale_and_recovers_on_heartbeat() -> Result<()> {
    ensure_control_plane_binaries_built()?;
    let runtime_registry_path = temp_path("fireline-control-plane-runtimes");
    let peer_directory_path = temp_path("fireline-control-plane-peers");
    let base_url = format!("http://127.0.0.1:{}", reserve_port()?);
    let registry = RuntimeRegistry::load(runtime_registry_path.clone())?;
    registry.upsert(seed_runtime("runtime:test"))?;

    let mut control_plane = spawn_control_plane(
        &base_url,
        &runtime_registry_path,
        &peer_directory_path,
        false,
        50,
        200,
    )
    .await?;

    let result = async {
        let client = reqwest::Client::new();
        let token = issue_runtime_token(&client, &base_url, "runtime:test").await?;

        let register_response = client
            .post(format!("{base_url}/v1/runtimes/runtime:test/register"))
            .bearer_auth(&token)
            .json(&registration_body())
            .send()
            .await?;
        assert_eq!(register_response.status(), StatusCode::OK);

        let ready = wait_for_status(&base_url, "runtime:test", RuntimeStatus::Ready).await?;
        assert_eq!(ready.runtime_id, "fireline:test:1");

        let stale = wait_for_status(&base_url, "runtime:test", RuntimeStatus::Stale).await?;
        assert_eq!(stale.status, RuntimeStatus::Stale);

        let heartbeat_response = client
            .post(format!("{base_url}/v1/runtimes/runtime:test/heartbeat"))
            .bearer_auth(&token)
            .json(&json!({
                "tsMs": 123_456,
                "metrics": null
            }))
            .send()
            .await?;
        assert_eq!(heartbeat_response.status(), StatusCode::OK);

        let recovered = wait_for_status(&base_url, "runtime:test", RuntimeStatus::Ready).await?;
        assert_eq!(recovered.updated_at_ms, 123_456);
        Ok(())
    }
    .await;

    shutdown_process(&mut control_plane).await;
    result
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

async fn spawn_control_plane(
    base_url: &str,
    runtime_registry_path: &PathBuf,
    peer_directory_path: &PathBuf,
    prefer_push: bool,
    heartbeat_scan_interval_ms: u64,
    stale_timeout_ms: u64,
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
        .arg("--startup-timeout-ms")
        .arg("20000")
        .arg("--stop-timeout-ms")
        .arg("10000")
        .arg("--heartbeat-scan-interval-ms")
        .arg(heartbeat_scan_interval_ms.to_string())
        .arg("--stale-timeout-ms")
        .arg(stale_timeout_ms.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if prefer_push {
        command.arg("--prefer-push");
    }

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

async fn wait_for_status(
    base_url: &str,
    runtime_key: &str,
    expected: RuntimeStatus,
) -> Result<RuntimeDescriptor> {
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
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
        tokio::time::sleep(Duration::from_millis(50)).await;
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

fn reserve_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .context("bind ephemeral port")?;
    Ok(listener.local_addr()?.port())
}

fn registration_body() -> serde_json::Value {
    json!({
        "runtimeId": "fireline:test:1",
        "nodeId": "node:test",
        "provider": "local",
        "providerInstanceId": "instance:test",
        "advertisedAcpUrl": "ws://127.0.0.1:4000/acp",
        "advertisedStateStreamUrl": "http://127.0.0.1:4000/v1/stream/fireline",
        "helperApiBaseUrl": null
    })
}

fn seed_runtime(runtime_key: &str) -> RuntimeDescriptor {
    RuntimeDescriptor {
        runtime_key: runtime_key.to_string(),
        runtime_id: String::new(),
        node_id: "node:seed".to_string(),
        provider: RuntimeProviderKind::Local,
        provider_instance_id: runtime_key.to_string(),
        status: RuntimeStatus::Starting,
        acp: Endpoint::new(""),
        state: Endpoint::new(""),
        helper_api_base_url: None,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}
