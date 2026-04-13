//! Local resumable ACP test agent for Fireline slice-08 testing.
//!
//! This is intentionally based on the SDK's `agent_client_protocol_test::testy`
//! behavior, but it advertises and implements `session/load` so Fireline can
//! prove runtime-owned terminal reattach without waiting on an upstream test
//! agent change.
//!
//! When `FIRELINE_ADVERTISED_STATE_STREAM_URL` is set in the environment,
//! `session/load` falls back to a durable-stream lookup against fireline's
//! session envelope history. This lets a cold-started testy_load rebuild its
//! session knowledge from the log and satisfy Anthropic's orchestration
//! `resume(sessionId)` contract without holding semantic agent state across
//! restarts. When the env var is unset, testy_load stays in memory-only
//! mode, which is the shape `tests/session_load_local.rs` pins for local
//! bootstrap restarts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_client_protocol_test::testy::TestyCommand;
use anyhow::Result;
use durable_streams::{Client as DurableStreamsClient, LiveMode, Offset};
use fireline_session::SessionRecord;
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
const STATE_STREAM_URL_ENV: &str = "FIRELINE_ADVERTISED_STATE_STREAM_URL";

#[derive(Clone, Debug)]
struct SessionData {
    #[allow(dead_code)]
    mcp_servers: Vec<McpServer>,
}

#[derive(Clone, Debug, Default)]
struct ResumableTesty {
    sessions: Arc<Mutex<HashMap<SessionId, SessionData>>>,
    state_stream_url: Option<String>,
}

impl ResumableTesty {
    fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            state_stream_url: std::env::var(STATE_STREAM_URL_ENV).ok(),
        }
    }

    fn create_session(&self, session_id: &SessionId, mcp_servers: Vec<McpServer>) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.insert(session_id.clone(), SessionData { mcp_servers });
    }

    fn has_session(&self, session_id: &SessionId) -> bool {
        self.sessions.lock().unwrap().contains_key(session_id)
    }

    async fn rebuild_session_from_stream(&self, session_id: &SessionId) -> Result<bool> {
        let Some(state_stream_url) = self.state_stream_url.as_deref() else {
            return Ok(false);
        };
        let target = session_id.to_string();
        let client = DurableStreamsClient::new();
        let stream = client.stream(state_stream_url);
        let mut reader = stream
            .read()
            .offset(Offset::Beginning)
            .live(LiveMode::Off)
            .build()?;
        while let Some(chunk) = reader.next_chunk().await? {
            if !chunk.data.is_empty() {
                let events: Vec<serde_json::Value> = serde_json::from_slice(&chunk.data)?;
                for event in events {
                    if event.get("type").and_then(|v| v.as_str()) != Some("session_v2") {
                        continue;
                    }
                    let Some(value) = event.get("value") else {
                        continue;
                    };
                    if let Ok(record) = serde_json::from_value::<SessionRecord>(value.clone())
                        && record.session_id.to_string() == target
                    {
                        self.create_session(session_id, Vec::new());
                        return Ok(true);
                    }
                }
            }
            if chunk.up_to_date {
                break;
            }
        }
        Ok(false)
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
                            return responder.respond(LoadSessionResponse::new());
                        }
                        match agent.rebuild_session_from_stream(&request.session_id).await {
                            Ok(true) => responder.respond(LoadSessionResponse::new()),
                            Ok(false) | Err(_) => responder
                                .respond_with_error(session_not_found_error(&request.session_id)),
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
