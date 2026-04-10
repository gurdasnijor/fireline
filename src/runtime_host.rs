//! Runtime/provider lifecycle surface.
//!
//! This module wraps the existing in-process bootstrap path in a
//! provider-oriented API with pinned runtime descriptors. The first
//! implementation only supports the `local` provider, but the public
//! shape is intentionally provider-agnostic so Flamecast and future
//! TS `client.host` APIs can target one stable runtime record model.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::bootstrap::{BootstrapConfig, BootstrapHandle};
use crate::runtime_registry::RuntimeRegistry;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProviderRequest {
    Auto,
    Local,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProviderKind {
    Local,
}

impl RuntimeProviderRequest {
    fn resolve(self) -> RuntimeProviderKind {
        match self {
            Self::Auto | Self::Local => RuntimeProviderKind::Local,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStatus {
    Starting,
    Ready,
    Busy,
    Idle,
    Stale,
    Broken,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDescriptor {
    pub runtime_key: String,
    pub runtime_id: String,
    pub node_id: String,
    pub provider: RuntimeProviderKind,
    pub provider_instance_id: String,
    pub status: RuntimeStatus,
    pub acp_url: String,
    pub state_stream_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helper_api_base_url: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct CreateRuntimeSpec {
    pub provider: RuntimeProviderRequest,
    pub host: IpAddr,
    pub port: u16,
    pub name: String,
    pub agent_command: Vec<String>,
    pub state_stream: Option<String>,
    pub peer_directory_path: Option<PathBuf>,
}

#[derive(Clone)]
pub struct RuntimeHost {
    inner: Arc<RuntimeHostInner>,
}

struct RuntimeHostInner {
    registry: RuntimeRegistry,
    live_handles: Mutex<HashMap<String, BootstrapHandle>>,
}

impl RuntimeHost {
    pub fn new(registry: RuntimeRegistry) -> Self {
        Self {
            inner: Arc::new(RuntimeHostInner {
                registry,
                live_handles: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn with_default_registry() -> Result<Self> {
        Ok(Self::new(RuntimeRegistry::load(
            RuntimeRegistry::default_path()?,
        )?))
    }

    pub async fn create(&self, spec: CreateRuntimeSpec) -> Result<RuntimeDescriptor> {
        let provider = spec.provider.resolve();
        let runtime_key = format!("runtime:{}", Uuid::new_v4());
        let created_at_ms = now_ms();
        let node_id = node_id_for(spec.host);

        self.inner.registry.upsert(RuntimeDescriptor {
            runtime_key: runtime_key.clone(),
            runtime_id: String::new(),
            node_id: node_id.clone(),
            provider,
            provider_instance_id: runtime_key.clone(),
            status: RuntimeStatus::Starting,
            acp_url: String::new(),
            state_stream_url: String::new(),
            helper_api_base_url: None,
            created_at_ms,
            updated_at_ms: created_at_ms,
        })?;

        let handle = start_local_runtime(spec).await?;

        let descriptor = RuntimeDescriptor {
            runtime_key: runtime_key.clone(),
            runtime_id: handle.runtime_id.clone(),
            node_id,
            provider,
            provider_instance_id: handle.runtime_id.clone(),
            status: RuntimeStatus::Ready,
            acp_url: handle.acp_url.clone(),
            state_stream_url: handle.state_stream_url.clone(),
            helper_api_base_url: None,
            created_at_ms,
            updated_at_ms: now_ms(),
        };

        self.inner.registry.upsert(descriptor.clone())?;
        self.inner
            .live_handles
            .lock()
            .await
            .insert(runtime_key, handle);

        Ok(descriptor)
    }

    pub fn get(&self, runtime_key: &str) -> Result<Option<RuntimeDescriptor>> {
        self.inner.registry.get(runtime_key)
    }

    pub fn list(&self) -> Result<Vec<RuntimeDescriptor>> {
        self.inner.registry.list()
    }

    pub async fn stop(&self, runtime_key: &str) -> Result<RuntimeDescriptor> {
        let handle = self
            .inner
            .live_handles
            .lock()
            .await
            .remove(runtime_key)
            .ok_or_else(|| anyhow!("runtime '{runtime_key}' is not running"))?;

        handle.shutdown().await?;

        let mut descriptor = self
            .inner
            .registry
            .get(runtime_key)?
            .ok_or_else(|| anyhow!("runtime '{runtime_key}' not found"))?;
        descriptor.status = RuntimeStatus::Stopped;
        descriptor.updated_at_ms = now_ms();
        self.inner.registry.upsert(descriptor.clone())?;
        Ok(descriptor)
    }

    pub async fn delete(&self, runtime_key: &str) -> Result<Option<RuntimeDescriptor>> {
        if self
            .inner
            .live_handles
            .lock()
            .await
            .contains_key(runtime_key)
        {
            self.stop(runtime_key).await?;
        }

        self.inner.registry.remove(runtime_key)
    }
}

async fn start_local_runtime(spec: CreateRuntimeSpec) -> Result<BootstrapHandle> {
    crate::bootstrap::start(BootstrapConfig {
        host: spec.host,
        port: spec.port,
        name: spec.name,
        runtime_key: None,
        node_id: None,
        agent_command: spec.agent_command,
        state_stream: spec.state_stream,
        peer_directory_path: spec.peer_directory_path,
    })
    .await
    .context("start local runtime")
}

fn node_id_for(host: IpAddr) -> String {
    if host.is_unspecified() {
        "node:local".to_string()
    } else {
        format!("node:{host}")
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}
