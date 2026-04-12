use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
pub use fireline_session::{
    ProvisionSpec, Endpoint, HeartbeatMetrics, HeartbeatReport, PersistedHostSpec,
    HostDescriptor, SandboxProviderKind, SandboxProviderRequest, HostRegistration,
    HostStatus,
};

pub struct SandboxLaunch {
    pub host_id: String,
    pub provider_instance_id: String,
    pub acp: Endpoint,
    pub state: Endpoint,
    pub helper_api_base_url: Option<String>,
    pub sandbox: Box<dyn ManagedSandbox>,
}

impl SandboxLaunch {
    pub fn ready(
        host_id: String,
        provider_instance_id: String,
        acp: Endpoint,
        state: Endpoint,
        helper_api_base_url: Option<String>,
        sandbox: Box<dyn ManagedSandbox>,
    ) -> Self {
        Self {
            host_id,
            provider_instance_id,
            acp,
            state,
            helper_api_base_url,
            sandbox,
        }
    }

    pub fn starting(sandbox: Box<dyn ManagedSandbox>) -> Self {
        Self {
            host_id: String::new(),
            provider_instance_id: String::new(),
            acp: Endpoint::new(""),
            state: Endpoint::new(""),
            helper_api_base_url: None,
            sandbox,
        }
    }
}

#[async_trait]
pub trait ManagedSandbox: Send {
    async fn shutdown(self: Box<Self>) -> Result<()>;
}

pub trait SandboxTokenIssuer: Send + Sync {
    fn issue(&self, host_key: &str, ttl: Duration) -> String;
}

#[async_trait]
pub trait SandboxProvider: Send + Sync {
    fn kind(&self) -> SandboxProviderKind;

    async fn provision(
        &self,
        spec: ProvisionSpec,
        host_key: String,
        node_id: String,
    ) -> Result<SandboxLaunch>;
}
