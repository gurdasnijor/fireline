use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};

use crate::provider::{
    ProvisionSpec, RuntimeLaunch, RuntimeProvider, SandboxProviderKind, SandboxProviderRequest,
};

#[derive(Clone)]
pub struct RuntimeManager {
    providers: HashMap<SandboxProviderKind, Arc<dyn RuntimeProvider>>,
}

impl RuntimeManager {
    pub fn new(local: Arc<dyn RuntimeProvider>) -> Self {
        let mut providers = HashMap::new();
        providers.insert(SandboxProviderKind::Local, local);
        Self { providers }
    }

    pub fn with_provider(mut self, provider: Arc<dyn RuntimeProvider>) -> Self {
        self.providers.insert(provider.kind(), provider);
        self
    }

    pub async fn start(
        &self,
        spec: ProvisionSpec,
        host_key: String,
        node_id: String,
    ) -> Result<(SandboxProviderKind, RuntimeLaunch)> {
        let provider = self.resolve(spec.provider)?;
        let kind = provider.kind();
        let launch = provider.start(spec, host_key, node_id).await?;
        Ok((kind, launch))
    }

    pub fn resolve_kind(&self, request: SandboxProviderRequest) -> Result<SandboxProviderKind> {
        self.resolve(request).map(|provider| provider.kind())
    }

    fn resolve(&self, request: SandboxProviderRequest) -> Result<Arc<dyn RuntimeProvider>> {
        match request {
            SandboxProviderRequest::Auto | SandboxProviderRequest::Local => self
                .providers
                .get(&SandboxProviderKind::Local)
                .cloned()
                .ok_or_else(|| anyhow!("local runtime provider is not configured")),
            SandboxProviderRequest::Docker => self
                .providers
                .get(&SandboxProviderKind::Docker)
                .cloned()
                .ok_or_else(|| anyhow!("docker runtime provider is not configured")),
        }
    }
}
