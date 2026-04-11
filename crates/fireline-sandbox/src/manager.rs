use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};

use crate::provider::{
    CreateRuntimeSpec, RuntimeLaunch, RuntimeProvider, RuntimeProviderKind, RuntimeProviderRequest,
};

#[derive(Clone)]
pub struct RuntimeManager {
    providers: HashMap<RuntimeProviderKind, Arc<dyn RuntimeProvider>>,
}

impl RuntimeManager {
    pub fn new(local: Arc<dyn RuntimeProvider>) -> Self {
        let mut providers = HashMap::new();
        providers.insert(RuntimeProviderKind::Local, local);
        Self { providers }
    }

    pub fn with_provider(mut self, provider: Arc<dyn RuntimeProvider>) -> Self {
        self.providers.insert(provider.kind(), provider);
        self
    }

    pub async fn start(
        &self,
        spec: CreateRuntimeSpec,
        runtime_key: String,
        node_id: String,
    ) -> Result<(RuntimeProviderKind, RuntimeLaunch)> {
        let provider = self.resolve(spec.provider)?;
        let kind = provider.kind();
        let launch = provider.start(spec, runtime_key, node_id).await?;
        Ok((kind, launch))
    }

    pub fn resolve_kind(&self, request: RuntimeProviderRequest) -> Result<RuntimeProviderKind> {
        self.resolve(request).map(|provider| provider.kind())
    }

    fn resolve(&self, request: RuntimeProviderRequest) -> Result<Arc<dyn RuntimeProvider>> {
        match request {
            RuntimeProviderRequest::Auto | RuntimeProviderRequest::Local => self
                .providers
                .get(&RuntimeProviderKind::Local)
                .cloned()
                .ok_or_else(|| anyhow!("local runtime provider is not configured")),
            RuntimeProviderRequest::Docker => self
                .providers
                .get(&RuntimeProviderKind::Docker)
                .cloned()
                .ok_or_else(|| anyhow!("docker runtime provider is not configured")),
        }
    }
}
