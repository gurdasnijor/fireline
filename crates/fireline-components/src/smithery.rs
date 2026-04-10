//! Smithery MCP bridge — SKETCH.
//!
//! Injects MCP servers hosted on [Smithery](https://smithery.ai) into
//! the agent's session by bridging to Smithery's stateless HTTP
//! endpoint. Each configured server is addressed by its `namespace`
//! and `connection_id`; the bridge POSTs MCP JSON-RPC requests to
//! `https://api.smithery.ai/connect/{namespace}/{connection_id}/mcp`
//! with `Authorization: Bearer <api_key>` and no session header.
//!
//! # SKETCH STATUS
//!
//! - Config, HTTP client construction, endpoint URL assembly, and a
//!   generic `raw_mcp_call` that POSTs JSON-RPC to Smithery are
//!   implemented.
//! - The `ConnectTo<Conductor>` impl is a pass-through proxy — the
//!   actual `with_mcp_server(...)` wiring for a Smithery-backed MCP
//!   server is TODO. The cleanest path (**Variant A**) is to depend
//!   on `agent-client-protocol-rmcp` and wrap
//!   `rmcp::transport::StreamableHttpClientTransport` via
//!   `McpServer::from_rmcp(...)`, which is not yet a workspace
//!   dependency. The fallback (**Variant B**) is to build an
//!   in-process `McpServer::builder(...)` with a single `smithery_call`
//!   dispatch tool, using a bootstrap `tools/list` call to discover
//!   available tools — this runs into a generic-type issue when
//!   iterating `.tool_fn(...)` calls in a loop.

use reqwest::Client as HttpClient;
use sacp::{ConnectTo, Proxy};

#[derive(Clone, Debug)]
pub struct SmitheryConfig {
    /// Smithery API key — typically sourced from the `SMITHERY_API_KEY`
    /// environment variable at bootstrap time.
    pub api_key: String,
    pub servers: Vec<SmitheryServerRef>,
}

#[derive(Clone, Debug)]
pub struct SmitheryServerRef {
    pub namespace: String,
    pub connection_id: String,
    /// Optional display name; defaults to `connection_id` if absent
    /// once the MCP server registration path is implemented.
    pub alias: Option<String>,
}

#[derive(Clone)]
pub struct SmitheryComponent {
    config: SmitheryConfig,
    http: HttpClient,
}

impl SmitheryComponent {
    pub fn new(config: SmitheryConfig) -> Self {
        Self {
            config,
            http: HttpClient::new(),
        }
    }

    /// Build the Smithery endpoint URL for a specific server.
    pub fn endpoint_for(&self, server: &SmitheryServerRef) -> String {
        format!(
            "https://api.smithery.ai/connect/{}/{}/mcp",
            server.namespace, server.connection_id
        )
    }

    /// Send a raw MCP JSON-RPC request to a Smithery-hosted server and
    /// return the parsed JSON response body.
    ///
    /// This is the primitive the eventual MCP tool adapter will use
    /// on every `tools/call` dispatch. It is kept public so a
    /// follow-up can prototype end-to-end calls before the full
    /// `with_mcp_server(...)` wiring lands.
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
}

#[derive(Debug)]
pub enum SmitheryError {
    Http(reqwest::Error),
    Status(u16),
}

impl std::fmt::Display for SmitheryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "smithery http error: {e}"),
            Self::Status(code) => write!(f, "smithery endpoint returned status {code}"),
        }
    }
}

impl std::error::Error for SmitheryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(e) => Some(e),
            Self::Status(_) => None,
        }
    }
}

impl ConnectTo<sacp::Conductor> for SmitheryComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        let _this = self;
        // TODO: on `NewSessionRequest`, build an `sacp::McpServer`
        // exposing one tool per Smithery tool (via bootstrap
        // `tools/list` against each configured server) and inject it
        // with `.with_mcp_server(...)`. Blocked today on:
        //   a) the generic-type issue with McpServer::builder that
        //      makes dynamic `.tool_fn(...)` registration in a loop
        //      awkward (Variant B), OR
        //   b) `agent-client-protocol-rmcp` not yet being a workspace
        //      dependency (Variant A — the clean path).
        sacp::Proxy
            .builder()
            .name("fireline-smithery")
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
