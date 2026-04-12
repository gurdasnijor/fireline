use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use fireline_resources::ResourceRef;
use fireline_session::{Endpoint, TopologySpec};
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait SandboxProvider: Send + Sync {
    fn name(&self) -> &str;

    fn capabilities(&self) -> ProviderCapabilities;

    async fn create(&self, config: &SandboxConfig) -> Result<SandboxHandle>;

    async fn get(&self, id: &str) -> Result<Option<SandboxDescriptor>>;

    async fn list(
        &self,
        labels: Option<&HashMap<String, String>>,
    ) -> Result<Vec<SandboxDescriptor>>;

    async fn execute(
        &self,
        id: &str,
        command: &str,
        timeout: Option<Duration>,
        env: Option<&HashMap<String, String>>,
    ) -> Result<ExecutionResult>;

    async fn destroy(&self, id: &str) -> Result<bool>;

    async fn health_check(&self) -> Result<bool>;

    async fn find(&self, labels: &HashMap<String, String>) -> Result<Option<SandboxDescriptor>> {
        Ok(self.list(Some(labels)).await?.into_iter().next())
    }

    async fn get_or_create(&self, config: &SandboxConfig) -> Result<SandboxHandle> {
        if let Some(existing) = self.find(&config.labels).await? {
            Ok(SandboxHandle::from_descriptor(existing, self.name()))
        } else {
            self.create(config).await
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxConfig {
    pub name: String,
    pub agent_command: Vec<String>,
    #[serde(default)]
    pub topology: TopologySpec,
    #[serde(default)]
    pub resources: Vec<ResourceRef>,
    pub durable_streams_url: String,
    pub state_stream: Option<String>,
    #[serde(default)]
    pub env_vars: HashMap<String, String>,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    pub provider: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxHandle {
    pub id: String,
    pub provider: String,
    pub acp: Endpoint,
    pub state: Endpoint,
}

impl SandboxHandle {
    pub fn from_descriptor(
        descriptor: SandboxDescriptor,
        default_provider: impl Into<String>,
    ) -> Self {
        let provider = if descriptor.provider.is_empty() {
            default_provider.into()
        } else {
            descriptor.provider
        };
        Self {
            id: descriptor.id,
            provider,
            acp: descriptor.acp,
            state: descriptor.state,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxDescriptor {
    pub id: String,
    pub provider: String,
    pub status: SandboxStatus,
    pub acp: Endpoint,
    pub state: Endpoint,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxStatus {
    Creating,
    Ready,
    Busy,
    Idle,
    Stopped,
    Broken,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub streaming: bool,
    pub file_transfer: bool,
    pub oci_images: bool,
    pub stream_resources: bool,
    pub snapshots: bool,
    pub gpu: bool,
    pub vm_isolation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub timed_out: bool,
}
