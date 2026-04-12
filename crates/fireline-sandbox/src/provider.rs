use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
pub use fireline_session::{
    CreateRuntimeSpec, Endpoint, HeartbeatMetrics, HeartbeatReport, PersistedRuntimeSpec,
    RuntimeDescriptor, RuntimeProviderKind, RuntimeProviderRequest, RuntimeRegistration,
    RuntimeStatus,
};

pub struct RuntimeLaunch {
    pub runtime_id: String,
    pub provider_instance_id: String,
    pub acp: Endpoint,
    pub state: Endpoint,
    pub helper_api_base_url: Option<String>,
    pub runtime: Box<dyn ManagedRuntime>,
}

impl RuntimeLaunch {
    pub fn ready(
        runtime_id: String,
        provider_instance_id: String,
        acp: Endpoint,
        state: Endpoint,
        helper_api_base_url: Option<String>,
        runtime: Box<dyn ManagedRuntime>,
    ) -> Self {
        Self {
            runtime_id,
            provider_instance_id,
            acp,
            state,
            helper_api_base_url,
            runtime,
        }
    }

    pub fn starting(runtime: Box<dyn ManagedRuntime>) -> Self {
        Self {
            runtime_id: String::new(),
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
    fn issue(&self, runtime_key: &str, ttl: Duration) -> String;
}

#[async_trait]
pub trait RuntimeProvider: Send + Sync {
    fn kind(&self) -> RuntimeProviderKind;

    async fn start(
        &self,
        spec: CreateRuntimeSpec,
        runtime_key: String,
        node_id: String,
    ) -> Result<RuntimeLaunch>;
}
