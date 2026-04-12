use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use fireline_resources::{LocalPathMounter, ResourceMounter, prepare_resources};

use crate::provider::{ProvisionSpec, RuntimeLaunch, RuntimeProvider, SandboxProviderKind};
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
    fn kind(&self) -> SandboxProviderKind {
        SandboxProviderKind::Local
    }

    async fn start(
        &self,
        spec: ProvisionSpec,
        host_key: String,
        node_id: String,
    ) -> Result<RuntimeLaunch> {
        let mounted_resources =
            prepare_resources(&spec.resources, &self.mounters, &host_key).await?;
        self.launcher
            .launch_local_runtime(spec, host_key, node_id, mounted_resources)
            .await
    }
}
