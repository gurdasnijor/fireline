use anyhow::{Context, Result};
use async_trait::async_trait;
use fireline_sandbox::{
    CreateRuntimeSpec, Endpoint, LocalRuntimeLauncher, ManagedRuntime, MountedResource,
    RuntimeLaunch,
};
use fireline_tools::LocalPeerDirectory;

use crate::bootstrap::{BootstrapConfig, BootstrapHandle};

pub struct BootstrapRuntimeLauncher;

#[async_trait]
impl LocalRuntimeLauncher for BootstrapRuntimeLauncher {
    async fn start_local_runtime(
        &self,
        spec: CreateRuntimeSpec,
        runtime_key: String,
        node_id: String,
        mounted_resources: Vec<MountedResource>,
    ) -> Result<RuntimeLaunch> {
        let peer_directory_path = match spec.peer_directory_path {
            Some(path) => path,
            None => LocalPeerDirectory::default_path()?,
        };

        let handle = crate::bootstrap::start(BootstrapConfig {
            host: spec.host,
            port: spec.port,
            name: spec.name,
            runtime_key,
            node_id,
            agent_command: spec.agent_command,
            mounted_resources,
            state_stream: spec.state_stream,
            stream_storage: spec.stream_storage,
            peer_directory_path,
            control_plane_url: None,
            external_state_stream_url: None,
            topology: spec.topology,
        })
        .await
        .context("start local runtime")?;

        Ok(RuntimeLaunch::ready(
            handle.runtime_id.clone(),
            handle.runtime_id.clone(),
            Endpoint::new(handle.acp_url.clone()),
            Endpoint::new(handle.state_stream_url.clone()),
            None,
            Box::new(handle),
        ))
    }
}

#[async_trait]
impl ManagedRuntime for BootstrapHandle {
    async fn shutdown(self: Box<Self>) -> Result<()> {
        (*self).shutdown().await
    }
}
