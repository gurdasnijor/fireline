use std::sync::Arc;

use anyhow::Result;
use fireline_conductor::runtime::{LocalProvider, RuntimeHost as InnerRuntimeHost, RuntimeManager};

use crate::runtime_provider::BootstrapRuntimeLauncher;
use crate::runtime_registry::RuntimeRegistry;

pub use fireline_conductor::runtime::{
    CreateRuntimeSpec, RuntimeDescriptor, RuntimeProviderKind, RuntimeProviderRequest,
    RuntimeStatus, StreamStorageConfig, StreamStorageMode,
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
        self.inner.create(spec).await
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
