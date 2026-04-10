//! [`PeerComponent`] — the [`sacp::component::Component`] implementation
//! that provides cross-agent calls for Fireline.

// TODO: implement PeerComponent
//
// Target shape (modeled on Sparkle's SparkleComponent):
//
// ```rust,ignore
// use sacp::component::Component;
// use sacp::{ProxyToConductor, ConductorToProxy, ClientPeer, AgentPeer};
// use sacp::schema::NewSessionRequest;
// use std::collections::HashMap;
// use std::path::PathBuf;
// use std::sync::{Arc, Mutex};
//
// use crate::directory::Directory;
// use crate::mcp_server::build_peer_mcp_server;
//
// pub struct PeerComponent {
//     directory_path: PathBuf,
// }
//
// impl PeerComponent {
//     pub fn new(directory_path: impl Into<PathBuf>) -> Self {
//         Self { directory_path: directory_path.into() }
//     }
// }
//
// impl Component<ProxyToConductor> for PeerComponent {
//     async fn serve(self, client: impl Component<ConductorToProxy>) -> Result<(), sacp::Error> {
//         let directory = Directory::load(&self.directory_path)?;
//         let session_lineage: Arc<Mutex<HashMap<String, ParentLineage>>> =
//             Arc::new(Mutex::new(HashMap::new()));
//
//         ProxyToConductor::builder()
//             .name("fireline-peer")
//             .on_receive_request_from(ClientPeer, {
//                 let directory = directory.clone();
//                 let session_lineage = session_lineage.clone();
//                 async move |request: NewSessionRequest, request_cx, connection_cx| {
//                     // Read incoming _meta.parent* from the initialize that
//                     // started this session (if it was a peer call).
//                     let parent_lineage = extract_parent_lineage(&request);
//
//                     // Build the MCP server for this session.
//                     let mcp_server = build_peer_mcp_server(
//                         directory.clone(),
//                         session_lineage.clone(),
//                     );
//
//                     // Inject the MCP server into the session via with_mcp_server.
//                     connection_cx
//                         .build_session_from(request)
//                         .with_mcp_server(mcp_server)?
//                         .on_proxy_session_start(request_cx, async move |session_id| {
//                             if let Some(lineage) = parent_lineage {
//                                 session_lineage.lock().unwrap().insert(session_id, lineage);
//                             }
//                             Ok(())
//                         })
//                 }
//             }, sacp::on_receive_request!())
//             .serve(client)
//             .await
//     }
// }
// ```
