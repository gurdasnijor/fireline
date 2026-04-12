use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use durable_streams::{Client as DsClient, Offset};
use fireline_harness::TopologySpec;
use fireline_runtime::LocalProvider;
use fireline_runtime::RuntimeRegistry;
use fireline_runtime::runtime_host::{
    CreateRuntimeSpec, RuntimeHost, RuntimeProviderKind, RuntimeProviderRequest, RuntimeStatus,
};
use fireline_runtime::{
    Endpoint, ManagedRuntime, RuntimeHost as ConductorRuntimeHost, RuntimeLaunch, RuntimeManager,
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
async fn runtime_host_pins_provider_and_persists_runtime_descriptor() -> Result<()> {
    let registry = RuntimeRegistry::load(temp_runtime_registry())?;
    let runtime_host = RuntimeHost::new(registry);
    let stream_server = stream_server::TestStreamServer::spawn().await?;

    let descriptor = runtime_host
        .create(CreateRuntimeSpec {
            runtime_key: None,
            node_id: None,
            provider: RuntimeProviderRequest::Auto,
            host: "127.0.0.1".parse::<IpAddr>()?,
            port: 0,
            name: "provider-test".to_string(),
            agent_command: vec![testy_bin()],
            durable_streams_url: stream_server.base_url.clone(),
            resources: Vec::new(),
            state_stream: None,
            stream_storage: None,
            peer_directory_path: Some(temp_peer_directory()),
            topology: TopologySpec::default(),
        })
        .await?;

    assert_eq!(descriptor.provider, RuntimeProviderKind::Local);
    assert_eq!(descriptor.status, RuntimeStatus::Ready);
    assert!(descriptor.runtime_key.starts_with("runtime:"));
    assert!(descriptor.runtime_id.starts_with("fireline:provider-test:"));
    assert!(descriptor.acp.url.starts_with("ws://"));
    assert!(descriptor.state.url.starts_with("http://"));
    assert_runtime_spec_event(&descriptor.state.url, &descriptor.runtime_key).await?;

    let listed = runtime_host.list()?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0], descriptor);

    let fetched = runtime_host.get(&descriptor.runtime_key)?;
    assert_eq!(fetched, Some(descriptor.clone()));

    let stopped = runtime_host.stop(&descriptor.runtime_key).await?;
    assert_eq!(stopped.status, RuntimeStatus::Stopped);

    let fetched_after_stop = runtime_host
        .get(&descriptor.runtime_key)?
        .expect("stopped descriptor should remain in the registry");
    assert_eq!(fetched_after_stop.status, RuntimeStatus::Stopped);

    let runtime_key = descriptor.runtime_key.clone();
    let deleted = runtime_host.delete(&runtime_key).await?;
    assert_eq!(
        deleted.map(|runtime| runtime.runtime_key),
        Some(runtime_key.clone())
    );
    assert!(runtime_host.get(&runtime_key)?.is_none());
    stream_server.shutdown().await;

    Ok(())
}

#[tokio::test]
async fn conductor_runtime_host_stays_starting_until_register_arrives() -> Result<()> {
    let registry = RuntimeRegistry::load(temp_runtime_registry())?;
    let runtime_host = ConductorRuntimeHost::new(
        registry,
        RuntimeManager::new(Arc::new(LocalProvider::new(Arc::new(FakeRuntimeLauncher)))),
    );

    let descriptor = runtime_host
        .create(CreateRuntimeSpec {
            runtime_key: None,
            node_id: None,
            provider: RuntimeProviderRequest::Local,
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

    assert_eq!(descriptor.status, RuntimeStatus::Starting);
    assert_eq!(descriptor.runtime_id, "fireline:pending-provider-test:fake");
    assert_eq!(descriptor.acp.url, "ws://127.0.0.1:4444/acp");
    assert_eq!(
        runtime_host
            .get(&descriptor.runtime_key)?
            .expect("descriptor should be persisted")
            .status,
        RuntimeStatus::Starting
    );

    let registered = runtime_host
        .register(
            &descriptor.runtime_key,
            fireline_runtime::RuntimeRegistration {
                runtime_id: descriptor.runtime_id.clone(),
                node_id: descriptor.node_id.clone(),
                provider: RuntimeProviderKind::Local,
                provider_instance_id: "fake-provider-instance".to_string(),
                advertised_acp_url: descriptor.acp.url.clone(),
                advertised_state_stream_url: descriptor.state.url.clone(),
                helper_api_base_url: None,
            },
        )
        .await?;
    assert_eq!(registered.status, RuntimeStatus::Ready);

    Ok(())
}

struct FakeRuntimeLauncher;

#[async_trait]
impl fireline_runtime::LocalRuntimeLauncher for FakeRuntimeLauncher {
    async fn start_local_runtime(
        &self,
        spec: CreateRuntimeSpec,
        _runtime_key: String,
        _node_id: String,
        _mounted_resources: Vec<fireline_runtime::MountedResource>,
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

async fn assert_runtime_spec_event(state_stream_url: &str, runtime_key: &str) -> Result<()> {
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
                if event.get("key").and_then(Value::as_str) != Some(runtime_key) {
                    continue;
                }

                let value = event
                    .get("value")
                    .and_then(Value::as_object)
                    .expect("runtime_spec row should carry a value object");
                assert_eq!(
                    value.get("runtimeKey").and_then(Value::as_str),
                    Some(runtime_key)
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
            anyhow::bail!("timed out waiting for runtime_spec event for {runtime_key}");
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
