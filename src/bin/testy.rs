//! SDK-backed ACP test agent for Fireline integration tests.
//!
//! This is a thin wrapper around `agent_client_protocol_test::testy::Testy`
//! so Fireline tests can exercise MCP tool discovery/calls without growing a
//! separate local test agent implementation.

use agent_client_protocol_test::testy::Testy;
use anyhow::Result;
use sacp::ConnectTo;

#[tokio::main]
async fn main() -> Result<()> {
    Testy::new().connect_to(sacp_tokio::Stdio::new()).await?;
    Ok(())
}
