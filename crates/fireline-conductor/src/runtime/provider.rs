use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::topology::TopologySpec;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProviderRequest {
    Auto,
    Local,
    Docker,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProviderKind {
    Local,
    Docker,
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
pub struct Endpoint {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
}

impl Endpoint {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: None,
        }
    }
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
    pub acp: Endpoint,
    pub state: Endpoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helper_api_base_url: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRegistration {
    pub runtime_id: String,
    pub node_id: String,
    pub provider: RuntimeProviderKind,
    pub provider_instance_id: String,
    pub advertised_acp_url: String,
    pub advertised_state_stream_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helper_api_base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatMetrics {
    pub active_sessions: u32,
    pub queue_depth: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatReport {
    pub ts_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<HeartbeatMetrics>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamStorageMode {
    Memory,
    FileFast,
    FileDurable,
    Acid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamStorageConfig {
    pub mode: StreamStorageMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acid_shard_count: Option<usize>,
}

impl StreamStorageConfig {
    pub fn file_durable(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            mode: StreamStorageMode::FileDurable,
            data_dir: Some(data_dir.into()),
            acid_shard_count: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateRuntimeSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub provider: RuntimeProviderRequest,
    pub host: IpAddr,
    pub port: u16,
    pub name: String,
    pub agent_command: Vec<String>,
    #[serde(default)]
    pub resources: Vec<ResourceRef>,
    pub state_stream: Option<String>,
    pub stream_storage: Option<StreamStorageConfig>,
    pub peer_directory_path: Option<PathBuf>,
    pub topology: TopologySpec,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersistedRuntimeSpec {
    pub runtime_key: String,
    pub node_id: String,
    #[serde(flatten)]
    pub create_spec: CreateRuntimeSpec,
}

impl PersistedRuntimeSpec {
    pub fn new(
        runtime_key: impl Into<String>,
        node_id: impl Into<String>,
        mut create_spec: CreateRuntimeSpec,
    ) -> Self {
        let runtime_key = runtime_key.into();
        let node_id = node_id.into();
        create_spec.runtime_key = Some(runtime_key.clone());
        create_spec.node_id = Some(node_id.clone());
        Self {
            runtime_key,
            node_id,
            create_spec,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResourceRef {
    LocalPath {
        path: PathBuf,
        mount_path: PathBuf,
    },
    GitRemote {
        repo_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reference: Option<String>,
        mount_path: PathBuf,
    },
    S3 {
        bucket: String,
        prefix: String,
        mount_path: PathBuf,
    },
    Gcs {
        bucket: String,
        prefix: String,
        mount_path: PathBuf,
    },
}

pub struct RuntimeLaunch {
    pub status: RuntimeStatus,
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
            status: RuntimeStatus::Ready,
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
            status: RuntimeStatus::Starting,
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
