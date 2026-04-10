//! Peer-call MCP server.
//!
//! Built per session and injected via `with_mcp_server` in
//! [`crate::component::PeerComponent`]. Exposes:
//!
//! - `list_peers`
//! - `prompt_peer`
//!
//! The actual peer wire is delegated to [`crate::transport`], which uses a
//! normal SDK ACP client session against the peer's hosted endpoint.

use std::sync::{Arc, OnceLock};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::directory::Directory;
use crate::transport;
use fireline_conductor::lineage::LineageTracker;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct ListPeersInput {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerInfo {
    pub runtime_id: String,
    pub agent_name: String,
    pub acp_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_stream_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct ListPeersOutput {
    pub peers: Vec<PeerInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptPeerInput {
    pub agent_name: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptPeerOutput {
    pub runtime_id: String,
    pub agent_name: String,
    pub response_text: String,
    pub stop_reason: String,
}

pub(crate) fn build_peer_mcp_server(
    directory: Directory,
    lineage_tracker: LineageTracker,
    session_binding: Arc<OnceLock<String>>,
) -> sacp::mcp_server::McpServer<Conductor, impl sacp::RunWithConnectionTo<Conductor>> {
    sacp::mcp_server::McpServer::builder("fireline-peer")
        .instructions("Discover and prompt peer Fireline runtimes over ACP.")
        .tool_fn(
            "list_peers",
            "List running Fireline peers available through the local directory.",
            {
                let directory = directory.clone();
                async move |_input: ListPeersInput, _cx| {
                    let peers = directory
                        .list()
                        .map_err(|e| sacp::util::internal_error(format!("list peers: {e}")))?
                        .into_iter()
                        .map(|peer| PeerInfo {
                            runtime_id: peer.runtime_id,
                            agent_name: peer.agent_name,
                            acp_url: peer.acp_url,
                            state_stream_url: peer.state_stream_url,
                        })
                        .collect();

                    Ok(ListPeersOutput { peers })
                }
            },
            sacp::tool_fn!(),
        )
        .tool_fn(
            "prompt_peer",
            "Send a prompt to a named Fireline peer and return its response.",
            {
                let directory = directory.clone();
                let lineage_tracker = lineage_tracker.clone();
                let session_binding = session_binding.clone();
                async move |input: PromptPeerInput, cx| {
                    let peer = directory
                        .lookup(&input.agent_name)
                        .map_err(|e| sacp::util::internal_error(format!("lookup peer: {e}")))?
                        .ok_or_else(|| {
                            sacp::util::internal_error(format!(
                                "peer '{}' not found",
                                input.agent_name
                            ))
                        })?;

                    let session_id = session_binding.get().cloned().ok_or_else(|| {
                        sacp::util::internal_error(format!(
                            "peer tool invoked before session binding was established ({})",
                            cx.acp_url()
                        ))
                    })?;

                    let parent_lineage =
                        lineage_tracker
                            .lineage_for_session(&session_id)
                            .map(|lineage| transport::ParentLineage {
                                trace_id: Some(lineage.trace_id),
                                parent_prompt_turn_id: Some(lineage.prompt_turn_id),
                            });

                    let result =
                        transport::dispatch_peer_call(&peer, &input.prompt, parent_lineage)
                            .await
                            .map_err(|e| {
                                sacp::util::internal_error(format!(
                                    "prompt peer '{}': {e}",
                                    input.agent_name
                                ))
                            })?;

                    Ok(PromptPeerOutput {
                        runtime_id: peer.runtime_id,
                        agent_name: peer.agent_name,
                        response_text: result.response_text,
                        stop_reason: result.stop_reason,
                    })
                }
            },
            sacp::tool_fn!(),
        )
        .build()
}

use sacp::Conductor;
