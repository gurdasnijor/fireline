#![forbid(unsafe_code)]

pub mod host_identity;
pub mod host_index;
pub mod projection;
pub mod state_materializer;
pub mod session_index;
pub mod stream_host;

pub use host_identity::{
    ProvisionSpec, Endpoint, HeartbeatMetrics, HeartbeatReport, PersistedHostSpec,
    HostDescriptor, SandboxProviderKind, SandboxProviderRequest, HostRegistration,
    HostStatus, TopologyComponentSpec, TopologySpec,
};
pub use host_index::{HostIndex, HostInstanceRecord, HostInstanceStatus};
pub use projection::{
    ChangeOperation, ControlKind, StateEnvelope, StateHeaders, StreamProjection,
};
pub use state_materializer::{StateMaterializer, StateMaterializerTask};
pub use session_index::SessionIndex;
pub use stream_host::*;

use serde::{Deserialize, Serialize};
use fireline_acp_ids::SessionId;

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
    pub session_id: SessionId,
    #[serde(rename = "runtimeKey")]
    pub host_key: String,
    #[serde(rename = "runtimeId")]
    pub host_id: String,
    pub node_id: String,
    pub state: SessionStatus,
    pub supports_load_session: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_seen_at: i64,
}
