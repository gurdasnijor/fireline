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

use super::transport;
use super::{PeerRegistry, resolve_prompt_trace_context};
use crate::ToolDescriptor;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct ListPeersInput {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerInfo {
    pub host_id: String,
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
    pub host_id: String,
    pub agent_name: String,
    pub response_text: String,
    pub stop_reason: String,
}

/// Return the `{name, description, input_schema}` triple for every tool
/// this component registers with its MCP server. Used by the topology
/// wire-up site to mirror the registered surface onto the durable state
/// stream as `tool_descriptor` envelopes so tests and external
/// subscribers can witness the Anthropic Tools primitive without
/// reaching through the MCP wire.
///
/// Keep this in sync with the `tool_fn(...)` chain in
/// [`build_peer_mcp_server`] — the description strings and input types
/// are the source of truth and live in exactly one place (here plus the
/// builder call below).
pub fn tool_descriptors() -> Vec<ToolDescriptor> {
    vec![
        ToolDescriptor {
            name: "list_peers".to_string(),
            description: "List running Fireline peers available through the deployment stream."
                .to_string(),
            input_schema: schemars::schema_for!(ListPeersInput).to_value(),
        },
        ToolDescriptor {
            name: "prompt_peer".to_string(),
            description: "Send a prompt to a named Fireline peer and return its response."
                .to_string(),
            input_schema: schemars::schema_for!(PromptPeerInput).to_value(),
        },
    ]
}

pub(crate) fn build_peer_mcp_server(
    peer_registry: Arc<dyn PeerRegistry>,
    session_binding: Arc<OnceLock<String>>,
) -> sacp::mcp_server::McpServer<Conductor, impl sacp::RunWithConnectionTo<Conductor>> {
    sacp::mcp_server::McpServer::builder("fireline-peer")
        .instructions("Discover and prompt peer Fireline runtimes over ACP.")
        .tool_fn(
            "list_peers",
            "List running Fireline peers available through the deployment stream.",
            {
                let peer_registry = peer_registry.clone();
                async move |_input: ListPeersInput, _cx| {
                    let peers = peer_registry
                        .list_peers()
                        .await
                        .map_err(|e| sacp::util::internal_error(format!("list peers: {e}")))?
                        .into_iter()
                        .map(|peer| PeerInfo {
                            host_id: peer.host_id,
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
                let peer_registry = peer_registry.clone();
                let session_binding = session_binding.clone();
                async move |input: PromptPeerInput, cx| {
                    let peer = peer_registry
                        .lookup_peer(&input.agent_name)
                        .await
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

                    let peer_call_span =
                        if let Some(prompt_context) = resolve_prompt_trace_context(&session_id) {
                            tracing::info_span!(
                                parent: &prompt_context.prompt_span,
                                "fireline.peer.call.out",
                                fireline.session_id = %session_id,
                                fireline.request_id = %prompt_context.request_id,
                                rpc.system = "jsonrpc",
                                rpc.method = "initialize",
                            )
                        } else {
                            tracing::info_span!(
                                "fireline.peer.call.out",
                                fireline.session_id = %session_id,
                                rpc.system = "jsonrpc",
                                rpc.method = "initialize",
                            )
                        };
                    let _peer_call_guard = peer_call_span.enter();
                    let trace_context = transport::TraceContextCarrier::from_current_span();

                    let result = transport::dispatch_peer_call(&peer, &input.prompt, trace_context)
                        .await
                        .map_err(|e| {
                            sacp::util::internal_error(format!(
                                "prompt peer '{}': {e}",
                                input.agent_name
                            ))
                        })?;

                    Ok(PromptPeerOutput {
                        host_id: peer.host_id,
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
