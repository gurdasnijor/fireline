use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use super::mounter::{LocalPathMounter, MountedResource, ResourceMounter, prepare_resources};
use super::provider::{CreateRuntimeSpec, RuntimeLaunch, RuntimeProvider, RuntimeProviderKind};

#[async_trait]
pub trait LocalRuntimeLauncher: Send + Sync {
    async fn start_local_runtime(
        &self,
        spec: CreateRuntimeSpec,
        runtime_key: String,
        node_id: String,
        mounted_resources: Vec<MountedResource>,
    ) -> Result<RuntimeLaunch>;
}

#[derive(Clone)]
pub struct LocalProvider {
    launcher: Arc<dyn LocalRuntimeLauncher>,
    mounters: Vec<Arc<dyn ResourceMounter>>,
}

impl LocalProvider {
    pub fn new(launcher: Arc<dyn LocalRuntimeLauncher>) -> Self {
        Self::with_mounters(launcher, vec![Arc::new(LocalPathMounter::new())])
    }

    pub fn with_mounters(
        launcher: Arc<dyn LocalRuntimeLauncher>,
        mounters: Vec<Arc<dyn ResourceMounter>>,
    ) -> Self {
        Self { launcher, mounters }
    }
}

#[async_trait]
impl RuntimeProvider for LocalProvider {
    fn kind(&self) -> RuntimeProviderKind {
        RuntimeProviderKind::Local
    }

    async fn start(
        &self,
        spec: CreateRuntimeSpec,
        runtime_key: String,
        node_id: String,
    ) -> Result<RuntimeLaunch> {
        let mounted_resources =
            prepare_resources(&spec.resources, &self.mounters, &runtime_key).await?;
        self.launcher
            .start_local_runtime(spec, runtime_key, node_id, mounted_resources)
            .await
    }
}
