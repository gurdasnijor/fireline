use std::net::IpAddr;
use std::path::PathBuf;

use anyhow::Result;
use fireline::runtime_host::{
    CreateRuntimeSpec, RuntimeHost, RuntimeProviderKind, RuntimeProviderRequest, RuntimeStatus,
};
use fireline::runtime_registry::RuntimeRegistry;
use uuid::Uuid;

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

    let descriptor = runtime_host
        .create(CreateRuntimeSpec {
            provider: RuntimeProviderRequest::Auto,
            host: "127.0.0.1".parse::<IpAddr>()?,
            port: 0,
            name: "provider-test".to_string(),
            agent_command: vec![testy_bin()],
            state_stream: None,
            peer_directory_path: Some(temp_peer_directory()),
        })
        .await?;

    assert_eq!(descriptor.provider, RuntimeProviderKind::Local);
    assert_eq!(descriptor.status, RuntimeStatus::Ready);
    assert!(descriptor.runtime_key.starts_with("runtime:"));
    assert!(descriptor.runtime_id.starts_with("fireline:provider-test:"));
    assert!(descriptor.acp_url.starts_with("ws://"));
    assert!(descriptor.state_stream_url.starts_with("http://"));

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

    Ok(())
}
