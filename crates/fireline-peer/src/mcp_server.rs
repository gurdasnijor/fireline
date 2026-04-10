//! Peer-call MCP server.
//!
//! Built per session and injected via `with_mcp_server` in
//! [`crate::component::PeerComponent`]. Exposes two tools to the
//! agent:
//!
//! - `list_peers` — returns the current peer directory contents
//! - `prompt_peer` — sends a prompt to a named peer agent and returns
//!   its response
//!
//! Both tools dispatch through [`crate::transport`] for the actual
//! peer wire. The MCP server is constructed per session so it can
//! capture session-specific context (like the current session's
//! lineage info, used to populate `_meta.parent*` on outgoing peer
//! calls).

// TODO: implement build_peer_mcp_server
//
// Target shape:
//
// ```rust,ignore
// use sacp::mcp_server::McpServer;
// use crate::directory::Directory;
//
// pub(crate) fn build_peer_mcp_server(
//     directory: Directory,
//     session_lineage: Arc<Mutex<HashMap<String, ParentLineage>>>,
// ) -> McpServer {
//     McpServer::builder("fireline-peer")
//         .instructions("Tools for discovering and prompting peer Fireline agents.")
//         .tool_fn("list_peers", "List all peer agents in the local directory.",
//             async move |_input: ListPeersInput, _cx| {
//                 directory.list().map_err(/* ... */)
//             }, sacp::tool_fn!(),
//         )
//         .tool_fn("prompt_peer", "Send a prompt to a named peer agent.",
//             async move |input: PromptPeerInput, _cx| {
//                 // Look up the target peer in the directory.
//                 // Look up THIS session's lineage in session_lineage.
//                 // Dispatch via crate::transport, stamping _meta.parent* with the lineage.
//                 // Return the peer's response.
//             }, sacp::tool_fn!(),
//         )
//         .build()
// }
// ```
