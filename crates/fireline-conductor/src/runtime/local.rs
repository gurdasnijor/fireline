use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use super::provider::{CreateRuntimeSpec, RuntimeLaunch, RuntimeProvider, RuntimeProviderKind};

#[async_trait]
pub trait LocalRuntimeLauncher: Send + Sync {
    async fn start_local_runtime(
        &self,
        spec: CreateRuntimeSpec,
        runtime_key: String,
        node_id: String,
    ) -> Result<RuntimeLaunch>;
}

#[derive(Clone)]
pub struct LocalProvider {
    launcher: Arc<dyn LocalRuntimeLauncher>,
}

impl LocalProvider {
    pub fn new(launcher: Arc<dyn LocalRuntimeLauncher>) -> Self {
        Self { launcher }
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
        self.launcher
            .start_local_runtime(spec, runtime_key, node_id)
            .await
    }
}
