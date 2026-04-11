use std::sync::Arc;

use anyhow::Result;
use crate::runtime::{LocalProvider, RuntimeHost as InnerRuntimeHost, RuntimeManager, RuntimeRegistry};
use crate::runtime_provider::BootstrapRuntimeLauncher;

pub use fireline_session::{
    CreateRuntimeSpec, Endpoint, RuntimeDescriptor, RuntimeProviderKind, RuntimeProviderRequest,
    RuntimeRegistration, RuntimeStatus, StreamStorageConfig, StreamStorageMode,
};

#[derive(Clone)]
pub struct RuntimeHost {
    inner: InnerRuntimeHost,
}

impl RuntimeHost {
    pub fn new(registry: RuntimeRegistry) -> Self {
        let launcher = Arc::new(BootstrapRuntimeLauncher);
        let local_provider = Arc::new(LocalProvider::new(launcher));
        let manager = RuntimeManager::new(local_provider);
        Self {
            inner: InnerRuntimeHost::new(registry, manager),
        }
    }

    pub fn with_default_registry() -> Result<Self> {
        Ok(Self::new(RuntimeRegistry::load(
            RuntimeRegistry::default_path()?,
        )?))
    }

    pub async fn create(&self, spec: CreateRuntimeSpec) -> Result<RuntimeDescriptor> {
        let descriptor = self.inner.create(spec).await?;
        if descriptor.status != RuntimeStatus::Starting {
            return Ok(descriptor);
        }

        self.inner
            .register(
            &descriptor.runtime_key,
            RuntimeRegistration {
                runtime_id: descriptor.runtime_id.clone(),
                node_id: descriptor.node_id.clone(),
                provider: descriptor.provider,
                provider_instance_id: descriptor.provider_instance_id.clone(),
                advertised_acp_url: descriptor.acp.url.clone(),
                advertised_state_stream_url: descriptor.state.url.clone(),
                helper_api_base_url: descriptor.helper_api_base_url.clone(),
            },
        )
            .await
    }

    pub fn get(&self, runtime_key: &str) -> Result<Option<RuntimeDescriptor>> {
        self.inner.get(runtime_key)
    }

    pub fn list(&self) -> Result<Vec<RuntimeDescriptor>> {
        self.inner.list()
    }

    pub async fn stop(&self, runtime_key: &str) -> Result<RuntimeDescriptor> {
        self.inner.stop(runtime_key).await
    }

    pub async fn delete(&self, runtime_key: &str) -> Result<Option<RuntimeDescriptor>> {
        self.inner.delete(runtime_key).await
    }
}
