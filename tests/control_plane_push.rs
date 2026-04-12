use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use fireline_session::{HostDescriptor, SandboxProviderKind, HostStatus};
use serde_json::json;
use tokio::process::{Child, Command};
use uuid::Uuid;

#[path = "support/stream_server.rs"]
mod stream_server;

fn fireline_bin() -> PathBuf {
    target_bin("fireline")
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
        let created = response.json::<HostDescriptor>().await?;
        let runtime =
            wait_for_status(&base_url, &created.host_key, HostStatus::Ready).await?;

        assert_eq!(runtime.status, HostStatus::Ready);
        assert_eq!(runtime.provider, SandboxProviderKind::Local);
        assert!(runtime.host_id.starts_with("fireline:push-test:"));
        assert_eq!(runtime.provider_instance_id, runtime.host_id);
        assert!(runtime.acp.url.starts_with("ws://"));
        assert!(runtime.state.url.starts_with("http://"));

        let stopped = client
            .post(format!(
                "{base_url}/v1/runtimes/{}/stop",
                runtime.host_key
            ))
            .send()
            .await?
            .error_for_status()?
            .json::<HostDescriptor>()
            .await?;
        assert_eq!(stopped.status, HostStatus::Stopped);

        let deleted = client
            .delete(format!("{base_url}/v1/runtimes/{}", runtime.host_key))
            .send()
            .await?
            .error_for_status()?
            .json::<HostDescriptor>()
            .await?;
        assert_eq!(deleted.host_key, runtime.host_key);
        Ok(())
    }
    .await;

    shutdown_process(&mut control_plane).await;
    shared_stream_server.shutdown().await;
    result
}

fn ensure_control_plane_binaries_built() -> Result<()> {
    let status = std::process::Command::new("cargo")
        .args(["build", "--quiet", "-p", "fireline", "--bins"])
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
    let mut command = Command::new(fireline_bin());
    command
        .arg("--control-plane")
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

    let mut child = command.spawn().context("spawn fireline --control-plane")?;
    wait_for_http_ok(&format!("{base_url}/healthz"), &mut child).await?;
    Ok(child)
}

async fn wait_for_status(
    base_url: &str,
    host_key: &str,
    expected: HostStatus,
) -> Result<HostDescriptor> {
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let response = client
            .get(format!("{base_url}/v1/runtimes/{host_key}"))
            .send()
            .await?;
        if response.status().is_success() {
            let descriptor = response.json::<HostDescriptor>().await?;
            if descriptor.status == expected {
                return Ok(descriptor);
            }
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for runtime '{host_key}' to become '{expected:?}'"
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
