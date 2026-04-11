#![forbid(unsafe_code)]

pub mod active_turn_index;
pub mod runtime_identity;
pub mod runtime_materializer;
pub mod session_index;
pub mod stream_host;

pub use active_turn_index::{ActiveTurnIndex, ActiveTurnRecord};
pub use runtime_identity::{
    CreateRuntimeSpec, Endpoint, HeartbeatMetrics, HeartbeatReport, PersistedRuntimeSpec,
    RuntimeDescriptor, RuntimeProviderKind, RuntimeProviderRequest, RuntimeRegistration,
    RuntimeStatus, TopologyComponentSpec, TopologySpec,
};
pub use runtime_materializer::{
    RawStateEnvelope, RawStateHeaders, RuntimeMaterializer, RuntimeMaterializerTask,
    StateProjection,
};
pub use session_index::SessionIndex;
pub use stream_host::*;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Broken,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub session_id: String,
    pub runtime_key: String,
    pub runtime_id: String,
    pub node_id: String,
    pub logical_connection_id: String,
    pub state: SessionStatus,
    pub supports_load_session: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_prompt_turn_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_seen_at: i64,
}
