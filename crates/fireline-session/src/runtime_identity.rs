use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::PathBuf;

use fireline_harness::TopologySpec;
use fireline_resources::ResourceRef;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::StreamStorageConfig;

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

#[derive(Debug, Clone, PartialEq)]
pub struct PersistedRuntimeSpec {
    pub runtime_key: String,
    pub node_id: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedRuntimeSpecWire {
    runtime_key: String,
    node_id: String,
    provider: RuntimeProviderRequest,
    host: IpAddr,
    port: u16,
    name: String,
    agent_command: Vec<String>,
    #[serde(default)]
    resources: Vec<ResourceRef>,
    state_stream: Option<String>,
    stream_storage: Option<StreamStorageConfig>,
    peer_directory_path: Option<PathBuf>,
    topology: TopologySpec,
}

impl Serialize for PersistedRuntimeSpec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        PersistedRuntimeSpecWire {
            runtime_key: self.runtime_key.clone(),
            node_id: self.node_id.clone(),
            provider: self.create_spec.provider,
            host: self.create_spec.host,
            port: self.create_spec.port,
            name: self.create_spec.name.clone(),
            agent_command: self.create_spec.agent_command.clone(),
            resources: self.create_spec.resources.clone(),
            state_stream: self.create_spec.state_stream.clone(),
            stream_storage: self.create_spec.stream_storage.clone(),
            peer_directory_path: self.create_spec.peer_directory_path.clone(),
            topology: self.create_spec.topology.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PersistedRuntimeSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = PersistedRuntimeSpecWire::deserialize(deserializer)?;
        Ok(Self {
            runtime_key: wire.runtime_key.clone(),
            node_id: wire.node_id.clone(),
            create_spec: CreateRuntimeSpec {
                runtime_key: Some(wire.runtime_key),
                node_id: Some(wire.node_id),
                provider: wire.provider,
                host: wire.host,
                port: wire.port,
                name: wire.name,
                agent_command: wire.agent_command,
                resources: wire.resources,
                state_stream: wire.state_stream,
                stream_storage: wire.stream_storage,
                peer_directory_path: wire.peer_directory_path,
                topology: wire.topology,
            },
        })
    }
}
