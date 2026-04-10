//! Minimal test ACP agent for integration tests.
//!
//! Speaks ACP over stdio. Responds to `initialize`, `session/new`,
//! and `session/prompt` with the simplest valid responses. Used by
//! integration tests as a deterministic stand-in for a real agent
//! (so tests don't depend on `npx`, `claude-acp`, or external
//! services).

use agent_client_protocol::{self as acp};
use anyhow::Result;
use sacp::{Client, ConnectTo};

struct TestAgent;

impl ConnectTo<Client> for TestAgent {
    async fn connect_to(self, client: impl ConnectTo<sacp::Agent>) -> Result<(), sacp::Error> {
        sacp::Agent
            .builder()
            .name("fireline-testy")
            .on_receive_request(
                async |req: acp::InitializeRequest, responder, _cx| {
                    responder.respond(acp::InitializeResponse::new(req.protocol_version))
                },
                sacp::on_receive_request!(),
            )
            .on_receive_request(
                async |_req: sacp::schema::NewSessionRequest, responder, _cx| {
                    responder.respond(sacp::schema::NewSessionResponse::new(acp::SessionId::new(
                        uuid::Uuid::new_v4().to_string(),
                    )))
                },
                sacp::on_receive_request!(),
            )
            .on_receive_request(
                async |_req: acp::PromptRequest, responder, _cx| {
                    responder.respond(acp::PromptResponse::new(acp::StopReason::EndTurn))
                },
                sacp::on_receive_request!(),
            )
            .connect_to(client)
            .await
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    TestAgent.connect_to(sacp_tokio::Stdio::new()).await?;
    Ok(())
}
