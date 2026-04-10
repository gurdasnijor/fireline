use std::sync::Arc;

use anyhow::Result;

use super::provider::{
    CreateRuntimeSpec, RuntimeLaunch, RuntimeProvider, RuntimeProviderKind, RuntimeProviderRequest,
};

#[derive(Clone)]
pub struct RuntimeManager {
    local: Arc<dyn RuntimeProvider>,
}

impl RuntimeManager {
    pub fn new(local: Arc<dyn RuntimeProvider>) -> Self {
        Self { local }
    }

    pub async fn start(
        &self,
        spec: CreateRuntimeSpec,
        runtime_key: String,
        node_id: String,
    ) -> Result<(RuntimeProviderKind, RuntimeLaunch)> {
        let provider = self.resolve(spec.provider);
        let kind = provider.kind();
        let launch = provider.start(spec, runtime_key, node_id).await?;
        Ok((kind, launch))
    }

    fn resolve(&self, request: RuntimeProviderRequest) -> Arc<dyn RuntimeProvider> {
        match request {
            RuntimeProviderRequest::Auto | RuntimeProviderRequest::Local => self.local.clone(),
        }
    }
}
