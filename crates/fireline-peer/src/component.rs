//! [`PeerComponent`] — the Fireline peer proxy component.
//!
//! This component does not manage ACP sessions itself. It stays on the SDK's
//! normal proxy/session path:
//!
//! - intercept `session/new`
//! - build the successor session with `build_session_from(...)`
//! - inject a per-session MCP server with `with_mcp_server(...)`
//! - hand the live session back to the SDK with `on_proxy_session_start(...)`

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use sacp::{Client, ConnectTo, Proxy};

use crate::directory::Directory;
use crate::lookup::{ActiveTurnLookup, ChildSessionEdgeSink};
use crate::mcp_server::build_peer_mcp_server;

#[derive(Clone)]
pub struct PeerComponent {
    directory_path: PathBuf,
    active_turn_lookup: Arc<dyn ActiveTurnLookup>,
    child_session_edge_sink: Arc<dyn ChildSessionEdgeSink>,
    runtime_id: String,
}

impl PeerComponent {
    pub fn new(
        directory_path: impl Into<PathBuf>,
        active_turn_lookup: Arc<dyn ActiveTurnLookup>,
        child_session_edge_sink: Arc<dyn ChildSessionEdgeSink>,
        runtime_id: impl Into<String>,
    ) -> Self {
        Self {
            directory_path: directory_path.into(),
            active_turn_lookup,
            child_session_edge_sink,
            runtime_id: runtime_id.into(),
        }
    }
}

impl ConnectTo<sacp::Conductor> for PeerComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        let directory = Directory::load(self.directory_path)
            .map_err(|e| sacp::util::internal_error(format!("load peer directory: {e}")))?;
        let active_turn_lookup = self.active_turn_lookup;
        let child_session_edge_sink = self.child_session_edge_sink;
        let runtime_id = self.runtime_id;

        sacp::Proxy
            .builder()
            .name("fireline-peer")
            .on_receive_request_from(
                Client,
                {
                    let directory = directory.clone();
                    let active_turn_lookup = active_turn_lookup.clone();
                    let child_session_edge_sink = child_session_edge_sink.clone();
                    let runtime_id = runtime_id.clone();
                    async move |request: sacp::schema::NewSessionRequest, responder, cx| {
                        let session_binding = Arc::new(OnceLock::new());
                        let mcp_server = build_peer_mcp_server(
                            directory.clone(),
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
