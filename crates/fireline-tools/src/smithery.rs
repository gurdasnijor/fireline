//! Smithery MCP bridge.
//!
//! Exposes MCP servers hosted on [Smithery](https://smithery.ai) to
//! an agent by injecting an in-process MCP server into the proxy
//! chain. The injected server exposes a single `smithery_call`
//! dispatch tool that takes `{server, tool, arguments}` and POSTs
//! the matching MCP `tools/call` JSON-RPC request to
//! `https://api.smithery.ai/connect/{namespace}/{connectionId}/mcp`
//! with `Authorization: Bearer <api_key>`.
//!
//! # Why a dispatch tool and not one tool per Smithery tool
//!
//! The obvious shape would be a bootstrap-time `tools/list` call
//! against each configured Smithery server that registers one
//! `.tool_fn(...)` per discovered tool. That runs into the
//! `McpServerBuilder` generic-type issue: each `.tool_fn(...)`
//! call changes the builder's generic type, so iterating over N
//! tools in a loop isn't expressible without the rmcp-based
//! Variant A path (see below). The dispatch tool sidesteps this
//! entirely: one `.tool_fn(...)` call, one well-known input
//! shape, any number of Smithery servers fan out at runtime.
//!
//! Trade-off: the agent sees one opaque tool (`smithery_call`)
//! rather than the proper tool surface of each underlying
//! server. For rich tool discovery you want Variant A.
//!
//! # Variant A (the clean path, deferred)
//!
//! The cookbook at `sacp::concepts::proxies` and
//! `agent-client-protocol-cookbook` shows the clean alternative:
//! use [`McpServer::from_rmcp`] from the
//! `agent-client-protocol-rmcp` crate with an
//! `rmcp::transport::StreamableHttpClientTransport` pointed at
//! the Smithery endpoint. The rmcp service provides proper
//! `tools/list` and `tools/call` behavior end-to-end.
//!
//! ```ignore
//! use agent_client_protocol_rmcp::McpServerExt;
//! use agent_client_protocol_core::{mcp_server::McpServer, Conductor};
//!
//! let mcp = McpServer::<Conductor, _>::from_rmcp("smithery", move || {
//!     // build an rmcp ServerHandler that wraps StreamableHttpClientTransport
//!     // against the Smithery endpoint with the API key header.
//!     todo!("rmcp service factory")
//! });
//! Proxy.builder().with_mcp_server(mcp).connect_to(transport).await
//! ```
//!
//! Blocked today on `agent-client-protocol-rmcp` not being a
//! workspace dependency of Fireline. The Variant B dispatch tool
//! below is useful on its own as a lightweight escape hatch
//! regardless — some Smithery endpoints might not even speak
//! full MCP.
//!
//! [`McpServer::from_rmcp`]: https://docs.rs/agent-client-protocol-rmcp

use durable_streams::Producer;
use reqwest::Client as HttpClient;
use sacp::{Conductor, ConnectTo, Proxy};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{ToolDescriptor, emit_tool_descriptors};

#[derive(Clone, Debug)]
pub struct SmitheryConfig {
    /// Smithery API key — typically sourced from the
    /// `SMITHERY_API_KEY` environment variable at bootstrap time.
    pub api_key: String,
    pub servers: Vec<SmitheryServerRef>,
}

#[derive(Clone, Debug)]
pub struct SmitheryServerRef {
    pub namespace: String,
    pub connection_id: String,
    /// Optional display name. When set, the agent can use this
    /// as the `server` field of `smithery_call` instead of
    /// `connection_id`.
    pub alias: Option<String>,
}

impl SmitheryServerRef {
    fn handle(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.connection_id)
    }
}

#[derive(Clone)]
pub struct SmitheryComponent {
    config: Arc<SmitheryConfig>,
    http: HttpClient,
    state_producer: Option<Producer>,
}

impl SmitheryComponent {
    pub fn new(config: SmitheryConfig) -> Self {
        Self {
            config: Arc::new(config),
            http: HttpClient::new(),
            state_producer: None,
        }
    }

    /// Attach a durable-streams producer so the component can emit
    /// `tool_descriptor` envelopes for every tool it registers with
    /// the agent. Without a producer the MCP wire-up still works —
    /// just without the state-stream projection. This is the hook the
    /// topology layer calls when it wants tests or external
    /// subscribers to witness the Anthropic triple.
    pub fn with_state_producer(mut self, producer: Producer) -> Self {
        self.state_producer = Some(producer);
        self
    }

    /// Return the Anthropic triple for every tool this component
    /// registers. Kept in sync with the `tool_fn(...)` chain in
    /// [`build_smithery_mcp_server`] — any new tool added below must
    /// add a matching descriptor here.
    pub fn tool_descriptors() -> Vec<ToolDescriptor> {
        vec![ToolDescriptor {
            name: "smithery_call".to_string(),
            description:
                "Call a tool on a configured Smithery-hosted MCP server."
                    .to_string(),
            input_schema: schemars::schema_for!(SmitheryCallInput).to_value(),
        }]
    }

    /// Build the Smithery endpoint URL for a specific server.
    pub fn endpoint_for(&self, server: &SmitheryServerRef) -> String {
        format!(
            "https://api.smithery.ai/connect/{}/{}/mcp",
            server.namespace, server.connection_id
        )
    }

    /// Resolve a user-supplied server handle (either the alias or
    /// the connection ID) to its [`SmitheryServerRef`].
    pub fn resolve_server(&self, handle: &str) -> Option<&SmitheryServerRef> {
        self.config.servers.iter().find(|s| s.handle() == handle)
    }

    /// Send a raw MCP JSON-RPC request to a Smithery-hosted
    /// server and return the parsed JSON response body.
    pub async fn raw_mcp_call(
        &self,
        server: &SmitheryServerRef,
        request: serde_json::Value,
    ) -> Result<serde_json::Value, SmitheryError> {
        let url = self.endpoint_for(server);
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(SmitheryError::Http)?;

        let status = resp.status();
        if !status.is_success() {
            return Err(SmitheryError::Status(status.as_u16()));
        }

        resp.json::<serde_json::Value>()
            .await
            .map_err(SmitheryError::Http)
    }

    /// Call a specific tool on a Smithery-hosted MCP server by
    /// name. Builds an MCP `tools/call` JSON-RPC request and
    /// delegates to [`Self::raw_mcp_call`].
    pub async fn call_tool(
        &self,
        server: &SmitheryServerRef,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, SmitheryError> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": "tools/call",
            "params": {
                "name": tool,
                "arguments": arguments,
            }
        });
        self.raw_mcp_call(server, request).await
    }
}

#[derive(Debug)]
pub enum SmitheryError {
    Http(reqwest::Error),
    Status(u16),
    Unknown(String),
}

impl std::fmt::Display for SmitheryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "smithery http error: {e}"),
            Self::Status(code) => write!(f, "smithery endpoint returned status {code}"),
            Self::Unknown(msg) => write!(f, "smithery error: {msg}"),
        }
    }
}

impl std::error::Error for SmitheryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(e) => Some(e),
            _ => None,
        }
    }
}

// ============================================================================
// MCP dispatch tool
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SmitheryCallInput {
    /// Name of the configured Smithery server (alias or connection id).
    pub server: String,
    /// Name of the tool to call on that server, as exposed by its MCP `tools/list`.
    pub tool: String,
    /// JSON arguments to pass to the tool. Defaults to `null` if omitted.
    #[serde(default)]
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SmitheryCallOutput {
    /// The raw JSON-RPC response body from the Smithery MCP endpoint.
    pub result: serde_json::Value,
}

fn build_smithery_mcp_server(
    component: SmitheryComponent,
) -> sacp::mcp_server::McpServer<Conductor, impl sacp::RunWithConnectionTo<Conductor>> {
    sacp::mcp_server::McpServer::builder("fireline-smithery")
        .instructions(
            "Dispatch tool calls to MCP servers hosted on Smithery. \
             Use smithery_call with {server, tool, arguments} to invoke a tool. \
             The set of configured servers and their aliases is determined by the \
             host runtime's Smithery configuration.",
        )
        .tool_fn(
            "smithery_call",
            "Call a tool on a configured Smithery-hosted MCP server.",
            {
                let component = component.clone();
                async move |input: SmitheryCallInput, _cx| {
                    let server = component.resolve_server(&input.server).ok_or_else(|| {
                        sacp::util::internal_error(format!(
                            "smithery_call: server '{}' is not configured",
                            input.server
                        ))
                    })?;
                    let result = component
                        .call_tool(server, &input.tool, input.arguments)
                        .await
                        .map_err(|e| {
                            sacp::util::internal_error(format!(
                                "smithery_call {}/{}: {e}",
                                input.server, input.tool
                            ))
                        })?;
                    Ok(SmitheryCallOutput { result })
                }
            },
            sacp::tool_fn!(),
        )
        .build()
}

impl ConnectTo<sacp::Conductor> for SmitheryComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        // Mirror the registered tool surface onto the durable state
        // stream before we wire the MCP server into the conductor
        // chain. Only runs when the topology layer attached a
        // producer; standalone construction (unit tests, etc.) stays
        // side-effect-free.
        if let Some(producer) = self.state_producer.as_ref() {
            let descriptors = Self::tool_descriptors();
            emit_tool_descriptors(producer, "smithery", &descriptors).await?;
        }
        let mcp_server = build_smithery_mcp_server(self);
        sacp::Proxy
            .builder()
            .name("fireline-smithery")
            .with_mcp_server(mcp_server)
            .connect_to(client)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_url_format() {
        let component = SmitheryComponent::new(SmitheryConfig {
            api_key: "test-key".to_string(),
            servers: vec![],
        });
        let server = SmitheryServerRef {
            namespace: "@smithery".to_string(),
            connection_id: "notion".to_string(),
            alias: None,
        };
        assert_eq!(
            component.endpoint_for(&server),
            "https://api.smithery.ai/connect/@smithery/notion/mcp"
        );
    }

    #[test]
    fn resolve_server_prefers_alias_then_connection_id() {
        let component = SmitheryComponent::new(SmitheryConfig {
            api_key: "dummy".to_string(),
            servers: vec![
                SmitheryServerRef {
                    namespace: "@smithery".to_string(),
                    connection_id: "notion".to_string(),
                    alias: Some("my-notion".to_string()),
                },
                SmitheryServerRef {
                    namespace: "@smithery".to_string(),
                    connection_id: "slack".to_string(),
                    alias: None,
                },
            ],
        });
        assert_eq!(
            component
                .resolve_server("my-notion")
                .map(|s| s.connection_id.as_str()),
            Some("notion")
        );
        assert_eq!(
            component
                .resolve_server("slack")
                .map(|s| s.connection_id.as_str()),
            Some("slack")
        );
        assert!(component.resolve_server("unknown").is_none());
    }

    #[test]
    fn smithery_call_input_deserializes_with_default_arguments() {
        let raw = r#"{"server":"my-notion","tool":"search_pages"}"#;
        let parsed: SmitheryCallInput = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.server, "my-notion");
        assert_eq!(parsed.tool, "search_pages");
        assert_eq!(parsed.arguments, serde_json::Value::Null);
    }

    #[test]
    fn smithery_component_constructs() {
        let _component = SmitheryComponent::new(SmitheryConfig {
            api_key: "dummy".to_string(),
            servers: vec![SmitheryServerRef {
                namespace: "@smithery".to_string(),
                connection_id: "test".to_string(),
                alias: Some("test-alias".to_string()),
            }],
        });
    }
}
