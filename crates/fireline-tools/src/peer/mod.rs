//! [`PeerComponent`] — the Fireline peer proxy component.
//!
//! This component does not manage ACP sessions itself. It stays on the SDK's
//! normal proxy/session path:
//!
//! - intercept `session/new`
//! - build the successor session with `build_session_from(...)`
//! - inject a per-session MCP server with `with_mcp_server(...)`
//! - hand the live session back to the SDK with `on_proxy_session_start(...)`

use anyhow::Result;
use async_trait::async_trait;

pub mod directory;
pub mod lookup;
pub mod stream;

pub(crate) mod mcp_server;
pub(crate) mod transport;

pub use mcp_server::tool_descriptors;
pub use stream::{
    DEFAULT_TENANT_ID, DeploymentDiscoveryEvent, DeploymentIndex, HostEntry, RuntimeEntry,
    StreamDeploymentPeerRegistry, deployment_stream_url,
};

use std::sync::{Arc, OnceLock};

use sacp::{Client, ConnectTo, Proxy};

use self::lookup::{ActiveTurnLookup, ChildSessionEdgeSink};
use self::mcp_server::build_peer_mcp_server;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Peer {
    pub runtime_id: String,
    pub agent_name: String,
    pub acp_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_stream_url: Option<String>,
    pub registered_at_ms: i64,
}

#[async_trait]
pub trait PeerRegistry: Send + Sync {
    async fn list_peers(&self) -> Result<Vec<Peer>>;

    async fn lookup_peer(&self, agent_name: &str) -> Result<Option<Peer>>;
}

#[derive(Clone)]
pub struct PeerComponent {
    peer_registry: Arc<dyn PeerRegistry>,
    active_turn_lookup: Arc<dyn ActiveTurnLookup>,
    child_session_edge_sink: Arc<dyn ChildSessionEdgeSink>,
    runtime_id: String,
}

impl PeerComponent {
    pub fn new(
        peer_registry: Arc<dyn PeerRegistry>,
        active_turn_lookup: Arc<dyn ActiveTurnLookup>,
        child_session_edge_sink: Arc<dyn ChildSessionEdgeSink>,
        runtime_id: impl Into<String>,
    ) -> Self {
        Self {
            peer_registry,
            active_turn_lookup,
            child_session_edge_sink,
            runtime_id: runtime_id.into(),
        }
    }
}

impl ConnectTo<sacp::Conductor> for PeerComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        let peer_registry = self.peer_registry;
        let active_turn_lookup = self.active_turn_lookup;
        let child_session_edge_sink = self.child_session_edge_sink;
        let runtime_id = self.runtime_id;

        sacp::Proxy
            .builder()
            .name("fireline-peer")
            .on_receive_request_from(
                Client,
                {
                    let peer_registry = peer_registry.clone();
                    let active_turn_lookup = active_turn_lookup.clone();
                    let child_session_edge_sink = child_session_edge_sink.clone();
                    let runtime_id = runtime_id.clone();
                    async move |request: sacp::schema::NewSessionRequest, responder, cx| {
                        let session_binding = Arc::new(OnceLock::new());
                        let mcp_server = build_peer_mcp_server(
                            peer_registry.clone(),
                            active_turn_lookup.clone(),
                            child_session_edge_sink.clone(),
                            runtime_id.clone(),
                            session_binding.clone(),
                        );
                        cx.build_session_from(request)
                            .with_mcp_server(mcp_server)?
                            .on_proxy_session_start(responder, async move |session_id| {
                                let _ = session_binding.set(session_id.to_string());
                                Ok(())
                            })
                    }
                },
                sacp::on_receive_request!(),
            )
            .connect_to(client)
            .await
    }
}
