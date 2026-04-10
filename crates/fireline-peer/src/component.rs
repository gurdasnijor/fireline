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

use fireline_conductor::lineage::LineageTracker;
use sacp::{Client, ConnectTo, Proxy};

use crate::directory::Directory;
use crate::mcp_server::build_peer_mcp_server;

#[derive(Debug, Clone)]
pub struct PeerComponent {
    directory_path: PathBuf,
    lineage_tracker: LineageTracker,
}

impl PeerComponent {
    pub fn new(directory_path: impl Into<PathBuf>, lineage_tracker: LineageTracker) -> Self {
        Self {
            directory_path: directory_path.into(),
            lineage_tracker,
        }
    }
}

impl ConnectTo<sacp::Conductor> for PeerComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        let directory = Directory::load(self.directory_path)
            .map_err(|e| sacp::util::internal_error(format!("load peer directory: {e}")))?;
        let lineage_tracker = self.lineage_tracker;

        sacp::Proxy
            .builder()
            .name("fireline-peer")
            .on_receive_request_from(
                Client,
                {
                    let directory = directory.clone();
                    let lineage_tracker = lineage_tracker.clone();
                    async move |request: sacp::schema::NewSessionRequest, responder, cx| {
                        let session_binding = Arc::new(OnceLock::new());
                        let mcp_server = build_peer_mcp_server(
                            directory.clone(),
                            lineage_tracker.clone(),
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
