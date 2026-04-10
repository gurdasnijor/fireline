//! Local resumable ACP test agent for Fireline slice-08 testing.
//!
//! This is intentionally based on the SDK's `agent_client_protocol_test::testy`
//! behavior, but it advertises and implements `session/load` so Fireline can
//! prove runtime-owned terminal reattach without waiting on an upstream test
//! agent change.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_client_protocol_test::testy::TestyCommand;
use anyhow::Result;
use sacp::schema::{
    AgentCapabilities, ContentBlock, ContentChunk, InitializeRequest, InitializeResponse,
    LoadSessionRequest, LoadSessionResponse, McpServer, NewSessionRequest, NewSessionResponse,
    PromptRequest, PromptResponse, SessionId, SessionNotification, SessionUpdate, StopReason,
    TextContent,
};
use sacp::{Agent, Client, ConnectTo, ConnectionTo, Responder};
use serde_json::json;

const SESSION_NOT_FOUND_CODE: i32 = -32061;
const SESSION_NOT_FOUND: &str = "session_not_found";

#[derive(Clone, Debug)]
struct SessionData {
    #[allow(dead_code)]
    mcp_servers: Vec<McpServer>,
}

#[derive(Clone, Debug, Default)]
struct ResumableTesty {
    sessions: Arc<Mutex<HashMap<SessionId, SessionData>>>,
}

impl ResumableTesty {
    fn new() -> Self {
        Self::default()
    }

    fn create_session(&self, session_id: &SessionId, mcp_servers: Vec<McpServer>) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.insert(session_id.clone(), SessionData { mcp_servers });
    }

    fn has_session(&self, session_id: &SessionId) -> bool {
        self.sessions.lock().unwrap().contains_key(session_id)
    }

    async fn process_prompt(
        &self,
        request: PromptRequest,
        responder: Responder<PromptResponse>,
        connection: ConnectionTo<Client>,
    ) -> Result<(), sacp::Error> {
        let session_id = request.session_id.clone();
        if !self.has_session(&session_id) {
            return responder.respond_with_error(session_not_found_error(&session_id));
        }

        let input_text = extract_text_from_prompt(&request.prompt);
        let command: TestyCommand =
            serde_json::from_str(&input_text).unwrap_or(TestyCommand::Greet);

        let response_text = match command {
            TestyCommand::Greet => "Hello, world!".to_string(),
            TestyCommand::Echo { message } => message,
            TestyCommand::CallTool { .. } => {
                "ERROR: fireline-testy-load does not implement MCP tool execution".to_string()
            }
            TestyCommand::ListTools { .. } => {
                "ERROR: fireline-testy-load does not implement MCP tool listing".to_string()
            }
        };

        connection.send_notification(SessionNotification::new(
            session_id,
            SessionUpdate::AgentMessageChunk(ContentChunk::new(response_text.into())),
        ))?;

        responder.respond(PromptResponse::new(StopReason::EndTurn))
    }
}

impl ConnectTo<Client> for ResumableTesty {
    async fn connect_to(self, client: impl ConnectTo<Agent>) -> Result<(), sacp::Error> {
        Agent
            .builder()
            .name("fireline-testy-load")
            .on_receive_request(
                async |initialize: InitializeRequest, responder, _cx| {
                    responder.respond(
                        InitializeResponse::new(initialize.protocol_version)
                            .agent_capabilities(AgentCapabilities::new().load_session(true)),
                    )
                },
                sacp::on_receive_request!(),
            )
            .on_receive_request(
                {
                    let agent = self.clone();
                    async move |request: NewSessionRequest, responder, _cx| {
                        let session_id = SessionId::new(uuid::Uuid::new_v4().to_string());
                        agent.create_session(&session_id, request.mcp_servers);
                        responder.respond(NewSessionResponse::new(session_id))
                    }
                },
                sacp::on_receive_request!(),
            )
            .on_receive_request(
                {
                    let agent = self.clone();
                    async move |request: LoadSessionRequest, responder, _cx| {
                        if agent.has_session(&request.session_id) {
                            responder.respond(LoadSessionResponse::new())
                        } else {
                            responder
                                .respond_with_error(session_not_found_error(&request.session_id))
                        }
                    }
                },
                sacp::on_receive_request!(),
            )
            .on_receive_request(
                {
                    let agent = self.clone();
                    async move |request: PromptRequest, responder, cx| {
                        let cx_clone = cx.clone();
                        cx.spawn({
                            let agent = agent.clone();
                            async move { agent.process_prompt(request, responder, cx_clone).await }
                        })
                    }
                },
                sacp::on_receive_request!(),
            )
            .connect_to(client)
            .await
    }
}

fn extract_text_from_prompt(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(TextContent { text, .. }) => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn session_not_found_error(session_id: &SessionId) -> sacp::Error {
    sacp::Error::new(SESSION_NOT_FOUND_CODE, SESSION_NOT_FOUND).data(json!({
        "sessionId": session_id,
    }))
}

#[tokio::main]
async fn main() -> Result<()> {
    ResumableTesty::new()
        .connect_to(sacp_tokio::Stdio::new())
        .await?;
    Ok(())
}
