use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use fireline_session::{HostDescriptor, HostIndex, HostStatus, SandboxProviderKind};

use crate::provider_model::{
    ProviderCapabilities, SandboxConfig, SandboxDescriptor, SandboxHandle, SandboxProvider,
    SandboxStatus,
};

#[derive(Clone)]
pub struct ProviderDispatcher {
    providers: Arc<HashMap<String, Arc<dyn SandboxProvider>>>,
    default_provider: Arc<str>,
    read_model: Arc<HostIndex>,
}

impl ProviderDispatcher {
    pub fn new(primary: Arc<dyn SandboxProvider>, read_model: Arc<HostIndex>) -> Self {
        let default_provider: Arc<str> = Arc::from(primary.name().to_string());
        let mut providers = HashMap::new();
        providers.insert(primary.name().to_string(), primary);
        Self {
            providers: Arc::new(providers),
            default_provider,
            read_model,
        }
    }

    pub fn with_provider(mut self, provider: Arc<dyn SandboxProvider>) -> Self {
        Arc::make_mut(&mut self.providers).insert(provider.name().to_string(), provider);
        self
    }

    pub async fn create(&self, config: SandboxConfig) -> Result<SandboxHandle> {
        let provider = self.select_provider(&config)?;
        provider.create(&config).await
    }

    pub async fn get(&self, id: &str) -> Result<Option<SandboxDescriptor>> {
        if let Some(descriptor) = self
            .read_model
            .endpoints_for(id)
            .await
            .map(sandbox_descriptor_from_host_descriptor)
        {
            return Ok(Some(descriptor));
        }

        for provider in self.providers.values() {
            if let Some(descriptor) = provider.get(id).await? {
                return Ok(Some(descriptor));
            }
        }

        Ok(None)
    }

    pub async fn list(
        &self,
        labels: Option<&HashMap<String, String>>,
    ) -> Result<Vec<SandboxDescriptor>> {
        let mut descriptors: Vec<_> = self
            .read_model
            .list_endpoints()
            .await
            .into_iter()
            .map(sandbox_descriptor_from_host_descriptor)
            .filter(|descriptor| labels_match(&descriptor.labels, labels))
            .collect();

        if descriptors.is_empty() {
            for provider in self.providers.values() {
                descriptors.extend(provider.list(labels).await?);
            }
            descriptors.sort_by(|left, right| left.id.cmp(&right.id));
            descriptors.dedup_by(|left, right| left.id == right.id);
        }

        Ok(descriptors)
    }

    pub async fn stop(&self, id: &str) -> Result<Option<SandboxDescriptor>> {
        let existing = self.get(id).await?;
        let Some(provider) = self.provider_for_id(id, existing.as_ref()).await? else {
            return Ok(None);
        };

        if !provider.destroy(id).await? {
            return Ok(None);
        }

        if let Some(updated) = provider.get(id).await? {
            return Ok(Some(updated));
        }
        if let Some(updated) = self.get(id).await? {
            return Ok(Some(updated));
        }

        Ok(existing.map(|mut descriptor| {
            descriptor.status = SandboxStatus::Stopped;
            descriptor.updated_at_ms = now_ms();
            descriptor
        }))
    }

    pub async fn health_check(&self) -> Result<HashMap<String, bool>> {
        let mut statuses = HashMap::with_capacity(self.providers.len());
        for (name, provider) in self.providers.iter() {
            statuses.insert(name.clone(), provider.health_check().await?);
        }
        Ok(statuses)
    }

    pub fn default_provider_name(&self) -> &str {
        self.default_provider.as_ref()
    }

    pub fn read_model(&self) -> &Arc<HostIndex> {
        &self.read_model
    }

    fn select_provider(&self, config: &SandboxConfig) -> Result<Arc<dyn SandboxProvider>> {
        let requested = config.provider.as_deref().unwrap_or(self.default_provider_name());
        let provider = self
            .providers
            .get(requested)
            .cloned()
            .ok_or_else(|| anyhow!("sandbox provider '{requested}' is not configured"))?;

        let capabilities = provider.capabilities();
        ensure_capabilities(provider.name(), &capabilities, config)?;

        Ok(provider)
    }

    async fn provider_for_id(
        &self,
        id: &str,
        descriptor: Option<&SandboxDescriptor>,
    ) -> Result<Option<Arc<dyn SandboxProvider>>> {
        if let Some(descriptor) = descriptor {
            if let Some(provider) = self.providers.get(&descriptor.provider) {
                return Ok(Some(provider.clone()));
            }
        }

        for provider in self.providers.values() {
            if provider.get(id).await?.is_some() {
                return Ok(Some(provider.clone()));
            }
        }

        Ok(None)
    }
}

fn sandbox_descriptor_from_host_descriptor(descriptor: HostDescriptor) -> SandboxDescriptor {
    SandboxDescriptor {
        id: descriptor.host_key,
        provider: provider_name(descriptor.provider).to_string(),
        status: sandbox_status_from_host_status(descriptor.status),
        acp: descriptor.acp,
        state: descriptor.state,
        labels: HashMap::new(),
        created_at_ms: descriptor.created_at_ms,
        updated_at_ms: descriptor.updated_at_ms,
    }
}

fn provider_name(kind: SandboxProviderKind) -> &'static str {
    match kind {
        SandboxProviderKind::Local => "local",
        SandboxProviderKind::Docker => "docker",
    }
}

fn sandbox_status_from_host_status(status: HostStatus) -> SandboxStatus {
    match status {
        HostStatus::Starting => SandboxStatus::Creating,
        HostStatus::Ready => SandboxStatus::Ready,
        HostStatus::Busy => SandboxStatus::Busy,
        HostStatus::Idle => SandboxStatus::Idle,
        HostStatus::Stopped => SandboxStatus::Stopped,
        HostStatus::Stale | HostStatus::Broken => SandboxStatus::Broken,
    }
}

fn labels_match(
    actual: &HashMap<String, String>,
    expected: Option<&HashMap<String, String>>,
) -> bool {
    let Some(expected) = expected else {
        return true;
    };

    expected
        .iter()
        .all(|(key, value)| actual.get(key).is_some_and(|actual_value| actual_value == value))
}

fn ensure_capabilities(
    provider_name: &str,
    capabilities: &ProviderCapabilities,
    config: &SandboxConfig,
) -> Result<()> {
    if !config.resources.is_empty() && !capabilities.file_transfer && !capabilities.stream_resources
    {
        return Err(anyhow!(
            "sandbox provider '{provider_name}' cannot mount requested resources"
        ));
    }

    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}
