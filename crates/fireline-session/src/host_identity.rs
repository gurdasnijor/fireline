use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::PathBuf;

use fireline_resources::ResourceRef;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::StreamStorageConfig;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxProviderRequest {
    Auto,
    Local,
    Docker,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SandboxProviderKind {
    Local,
    Docker,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostStatus {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TopologySpec {
    #[serde(default)]
    pub components: Vec<TopologyComponentSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TopologyComponentSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

impl Default for TopologySpec {
    fn default() -> Self {
        Self {
            components: vec![TopologyComponentSpec {
                name: "peer_mcp".to_string(),
                config: None,
            }],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostDescriptor {
    #[serde(rename = "runtimeKey")]
    pub host_key: String,
    #[serde(rename = "runtimeId")]
    pub host_id: String,
    pub node_id: String,
    pub provider: SandboxProviderKind,
    pub provider_instance_id: String,
    pub status: HostStatus,
    pub acp: Endpoint,
    pub state: Endpoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helper_api_base_url: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostRegistration {
    #[serde(rename = "runtimeId")]
    pub host_id: String,
    pub node_id: String,
    pub provider: SandboxProviderKind,
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
pub struct ProvisionSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "runtimeKey")]
    pub host_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub provider: SandboxProviderRequest,
    pub host: IpAddr,
    pub port: u16,
    pub name: String,
    pub agent_command: Vec<String>,
    pub durable_streams_url: String,
    #[serde(default)]
    pub resources: Vec<ResourceRef>,
    pub state_stream: Option<String>,
    pub stream_storage: Option<StreamStorageConfig>,
    pub peer_directory_path: Option<PathBuf>,
    pub topology: TopologySpec,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PersistedHostSpec {
    pub host_key: String,
    pub node_id: String,
    pub create_spec: ProvisionSpec,
}

impl PersistedHostSpec {
    pub fn new(
        host_key: impl Into<String>,
        node_id: impl Into<String>,
        mut create_spec: ProvisionSpec,
    ) -> Self {
        let host_key = host_key.into();
        let node_id = node_id.into();
        create_spec.host_key = Some(host_key.clone());
        create_spec.node_id = Some(node_id.clone());
        Self {
            host_key,
            node_id,
            create_spec,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedHostSpecWire {
    #[serde(rename = "runtimeKey")]
    host_key: String,
    node_id: String,
    provider: SandboxProviderRequest,
    host: IpAddr,
    port: u16,
    name: String,
    agent_command: Vec<String>,
    durable_streams_url: String,
    #[serde(default)]
    resources: Vec<ResourceRef>,
    state_stream: Option<String>,
    stream_storage: Option<StreamStorageConfig>,
    peer_directory_path: Option<PathBuf>,
    topology: TopologySpec,
}

impl Serialize for PersistedHostSpec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        PersistedHostSpecWire {
            host_key: self.host_key.clone(),
            node_id: self.node_id.clone(),
            provider: self.create_spec.provider,
            host: self.create_spec.host,
            port: self.create_spec.port,
            name: self.create_spec.name.clone(),
            agent_command: self.create_spec.agent_command.clone(),
            durable_streams_url: self.create_spec.durable_streams_url.clone(),
            resources: self.create_spec.resources.clone(),
            state_stream: self.create_spec.state_stream.clone(),
            stream_storage: self.create_spec.stream_storage.clone(),
            peer_directory_path: self.create_spec.peer_directory_path.clone(),
            topology: self.create_spec.topology.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PersistedHostSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = PersistedHostSpecWire::deserialize(deserializer)?;
        Ok(Self {
            host_key: wire.host_key.clone(),
            node_id: wire.node_id.clone(),
            create_spec: ProvisionSpec {
                host_key: Some(wire.host_key),
                node_id: Some(wire.node_id),
                provider: wire.provider,
                host: wire.host,
                port: wire.port,
                name: wire.name,
                agent_command: wire.agent_command,
                durable_streams_url: wire.durable_streams_url,
                resources: wire.resources,
                state_stream: wire.state_stream,
                stream_storage: wire.stream_storage,
                peer_directory_path: wire.peer_directory_path,
                topology: wire.topology,
            },
        })
    }
}
