use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use durable_streams::{Client as DsClient, Offset};
use fireline_harness::TopologySpec;
use fireline_host::bootstrap::{self, BootstrapConfig};
use fireline_sandbox::{
    ProvisionSpec, Endpoint, LocalProvider, LocalRuntimeLauncher, ManagedRuntime,
    MountedResource, RuntimeHost as SandboxRuntimeHost, RuntimeLaunch, RuntimeManager,
    SandboxProviderKind, SandboxProviderRequest, HostRegistration, RuntimeRegistry,
    HostStatus,
};
use serde_json::Value;
use uuid::Uuid;

#[path = "support/stream_server.rs"]
mod stream_server;

fn testy_bin() -> String {
    PathBuf::from(env!("CARGO_BIN_EXE_fireline-testy"))
        .display()
        .to_string()
}

fn temp_runtime_registry() -> PathBuf {
    std::env::temp_dir().join(format!("fireline-runtimes-{}.toml", Uuid::new_v4()))
}

fn temp_peer_directory() -> PathBuf {
    std::env::temp_dir().join(format!("fireline-peers-{}.toml", Uuid::new_v4()))
}

#[tokio::test]
async fn direct_host_bootstrap_emits_host_spec_and_exposes_host_endpoints() -> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let host_key = format!("runtime:{}", Uuid::new_v4());
    let node_id = "node:provider-test".to_string();

    let handle = bootstrap::start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "provider-test".to_string(),
        host_key: host_key.clone(),
        node_id,
        agent_command: vec![testy_bin()],
        mounted_resources: Vec::new(),
        state_stream: None,
        durable_streams_url: stream_server.base_url.clone(),
        peer_directory_path: temp_peer_directory(),
        control_plane_url: None,
        topology: TopologySpec::default(),
    })
    .await?;
    wait_for_health(&handle.health_url).await?;

    assert!(handle.host_id.starts_with("fireline:provider-test:"));
    assert!(handle.acp_url.starts_with("ws://"));
    assert!(handle.state_stream_url.starts_with("http://"));
    assert_host_spec_event(&handle.state_stream_url, &host_key).await?;

    handle.shutdown().await?;
    stream_server.shutdown().await;

    Ok(())
}

#[tokio::test]
async fn sandbox_runtime_host_stays_starting_until_register_arrives() -> Result<()> {
    let registry = RuntimeRegistry::load(temp_runtime_registry())?;
    let runtime_host = SandboxRuntimeHost::new(
        registry,
        RuntimeManager::new(Arc::new(LocalProvider::new(Arc::new(FakeRuntimeLauncher)))),
    );

    let descriptor = runtime_host
        .provision(ProvisionSpec {
            host_key: None,
            node_id: None,
            provider: SandboxProviderRequest::Local,
            host: "127.0.0.1".parse::<IpAddr>()?,
            port: 0,
            name: "pending-provider-test".to_string(),
            agent_command: vec![testy_bin()],
            durable_streams_url: "http://127.0.0.1:4444/v1/stream".to_string(),
            resources: Vec::new(),
            state_stream: None,
            stream_storage: None,
            peer_directory_path: Some(temp_peer_directory()),
            topology: TopologySpec::default(),
        })
        .await?;

    assert_eq!(descriptor.status, HostStatus::Starting);
    assert_eq!(descriptor.host_id, "fireline:pending-provider-test:fake");
    assert_eq!(descriptor.acp.url, "ws://127.0.0.1:4444/acp");
    assert_eq!(
        runtime_host
            .get(&descriptor.host_key)?
            .expect("descriptor should be persisted")
            .status,
        HostStatus::Starting
    );

    let registered = runtime_host
        .register(
            &descriptor.host_key,
            HostRegistration {
                host_id: descriptor.host_id.clone(),
                node_id: descriptor.node_id.clone(),
                provider: SandboxProviderKind::Local,
                provider_instance_id: "fake-provider-instance".to_string(),
                advertised_acp_url: descriptor.acp.url.clone(),
                advertised_state_stream_url: descriptor.state.url.clone(),
                helper_api_base_url: None,
            },
        )
        .await?;
    assert_eq!(registered.status, HostStatus::Ready);

    Ok(())
}

struct FakeRuntimeLauncher;

#[async_trait]
impl LocalRuntimeLauncher for FakeRuntimeLauncher {
    async fn launch_local_runtime(
        &self,
        spec: ProvisionSpec,
        _host_key: String,
        _node_id: String,
        _mounted_resources: Vec<MountedResource>,
    ) -> Result<RuntimeLaunch> {
        Ok(RuntimeLaunch::ready(
            format!("fireline:{}:fake", spec.name),
            "fake-provider-instance".to_string(),
            Endpoint::new("ws://127.0.0.1:4444/acp"),
            Endpoint::new("http://127.0.0.1:4444/v1/stream/fireline"),
            None,
            Box::new(FakeManagedRuntime),
        ))
    }
}

struct FakeManagedRuntime;

#[async_trait]
impl ManagedRuntime for FakeManagedRuntime {
    async fn shutdown(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}

async fn assert_host_spec_event(state_stream_url: &str, host_key: &str) -> Result<()> {
    let client = DsClient::new();
    let stream = client.stream(state_stream_url);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);

    loop {
        let mut reader = stream.read().offset(Offset::Beginning).build()?;
        let mut found = false;
        while let Some(chunk) = reader.next_chunk().await? {
            for event in serde_json::from_slice::<Vec<Value>>(&chunk.data)? {
                if event.get("type").and_then(Value::as_str) != Some("runtime_spec") {
                    continue;
                }
                if event.get("key").and_then(Value::as_str) != Some(host_key) {
                    continue;
                }

                let value = event
                    .get("value")
                    .and_then(Value::as_object)
                    .expect("host_spec row should carry a value object");
                assert_eq!(
                    value.get("runtimeKey").and_then(Value::as_str),
                    Some(host_key)
                );
                found = true;
                break;
            }

            if found {
                return Ok(());
            }
            if chunk.up_to_date {
                break;
            }
        }

        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for host_spec event for {host_key}");
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_health(health_url: &str) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);

    loop {
        match reqwest::get(health_url).await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(_) | Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(response) => anyhow::bail!("health check failed with status {}", response.status()),
            Err(error) => return Err(error.into()),
        }
    }
}
