use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
pub use fireline_session::{
    ProvisionSpec, Endpoint, HeartbeatMetrics, HeartbeatReport, PersistedHostSpec,
    HostDescriptor, SandboxProviderKind, SandboxProviderRequest, HostRegistration,
    HostStatus,
};

pub struct RuntimeLaunch {
    pub host_id: String,
    pub provider_instance_id: String,
    pub acp: Endpoint,
    pub state: Endpoint,
    pub helper_api_base_url: Option<String>,
    pub runtime: Box<dyn ManagedRuntime>,
}

impl RuntimeLaunch {
    pub fn ready(
        host_id: String,
        provider_instance_id: String,
        acp: Endpoint,
        state: Endpoint,
        helper_api_base_url: Option<String>,
        runtime: Box<dyn ManagedRuntime>,
    ) -> Self {
        Self {
            host_id,
            provider_instance_id,
            acp,
            state,
            helper_api_base_url,
            runtime,
        }
    }

    pub fn starting(runtime: Box<dyn ManagedRuntime>) -> Self {
        Self {
            host_id: String::new(),
            provider_instance_id: String::new(),
            acp: Endpoint::new(""),
            state: Endpoint::new(""),
            helper_api_base_url: None,
            runtime,
        }
    }
}

#[async_trait]
pub trait ManagedRuntime: Send {
    async fn shutdown(self: Box<Self>) -> Result<()>;
}

pub trait RuntimeTokenIssuer: Send + Sync {
    fn issue(&self, host_key: &str, ttl: Duration) -> String;
}

#[async_trait]
pub trait RuntimeProvider: Send + Sync {
    fn kind(&self) -> SandboxProviderKind;

    async fn start(
        &self,
        spec: ProvisionSpec,
        host_key: String,
        node_id: String,
    ) -> Result<RuntimeLaunch>;
}
