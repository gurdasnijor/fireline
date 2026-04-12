use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use fireline_sandbox::{SandboxDescriptor, SandboxHandle, SandboxStatus};
use fireline_session::{HostStatus, SandboxProviderKind};
use serde_json::json;
use tokio::process::{Child, Command};
use uuid::Uuid;

#[path = "support/control_plane_harness.rs"]
mod control_plane_harness;
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

async fn assert_runtime_lifecycle_round_trip(_prefer_push: bool) -> Result<()> {
    let peer_directory_path = temp_path("fireline-control-plane-peers");
    let shared_stream_server = stream_server::TestStreamServer::spawn().await?;
    let base_url = format!("http://127.0.0.1:{}", reserve_port()?);
    let mut control_plane = spawn_control_plane(
        &base_url,
        &peer_directory_path,
        &shared_stream_server.base_url,
    )
    .await?;

    let result = async {
        let client = reqwest::Client::new();
        let response = client
            .post(format!("{base_url}/v1/sandboxes"))
            .json(&json!({
                "provider": "local",
                "name": "push-test",
                "agentCommand": [testy_bin()],
                "topology": { "components": [] }
            }))
            .send()
            .await?
            .error_for_status()?;
        let created = response.json::<SandboxHandle>().await?;
        let runtime =
            control_plane_harness::wait_for_host_status(&base_url, &created.id, HostStatus::Ready)
                .await?;

        assert_eq!(runtime.status, HostStatus::Ready);
        assert_eq!(runtime.provider, SandboxProviderKind::Local);
        assert!(runtime.host_id.starts_with("fireline:push-test:"));
        assert_eq!(runtime.provider_instance_id, runtime.host_id);
        assert!(runtime.acp.url.starts_with("ws://"));
        assert!(runtime.state.url.starts_with("http://"));

        let stopped = client
            .post(format!("{base_url}/v1/sandboxes/{}/stop", runtime.host_key))
            .send()
            .await?
            .error_for_status()?
            .json::<SandboxDescriptor>()
            .await?;
        assert_eq!(stopped.status, SandboxStatus::Stopped);

        let deleted = client
            .delete(format!("{base_url}/v1/sandboxes/{}", runtime.host_key))
            .send()
            .await?;
        assert_eq!(deleted.status(), reqwest::StatusCode::NOT_FOUND);
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
    peer_directory_path: &PathBuf,
    durable_streams_url: &str,
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
        .arg("--peer-directory-path")
        .arg(peer_directory_path)
        .arg("--startup-timeout-ms")
        .arg("20000")
        .arg("--stop-timeout-ms")
        .arg("10000")
        .arg("--durable-streams-url")
        .arg(durable_streams_url)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let mut child = command.spawn().context("spawn fireline --control-plane")?;
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
