use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use fireline_resources::{LocalPathMounter, ResourceMounter, prepare_resources};

use crate::provider::{CreateRuntimeSpec, RuntimeLaunch, RuntimeProvider, RuntimeProviderKind};
use crate::provider_trait::LocalRuntimeLauncher;

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
