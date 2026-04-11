//! Scripted ACP test agent that deterministically emits ACP fs requests.
//!
//! `fireline-testy-fs` is a narrow extension of the SDK's `testy` pattern
//! that adds the ability to write or read a text file via ACP's
//! agent-to-client `fs/write_text_file` and `fs/read_text_file` requests
//! on command. Tests serialize a `FsTestyCommand` as prompt text; the
//! agent parses it and issues the matching ACP request against its
//! client connection, then responds to the prompt with the result.
//!
//! This exists because the upstream `TestyCommand` enum has no fs
//! emission paths, and several `tests/managed_agent_resources.rs`
//! invariants require the agent to actually emit `fs/write_text_file`
//! so that Fireline's `FsBackendComponent` can intercept it. Without a
//! scripted path those tests had to rely on the real (nondeterministic)
//! agent behavior.

use std::path::PathBuf;

use anyhow::Result;
use sacp::schema::{
    AgentCapabilities, ContentBlock, ContentChunk, InitializeRequest, InitializeResponse,
    NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse, ReadTextFileRequest,
    SessionId, SessionNotification, SessionUpdate, StopReason, TextContent, WriteTextFileRequest,
};
use sacp::{Agent, Client, ConnectTo, ConnectionTo, Responder};
use serde::{Deserialize, Serialize};

/// Commands the scripted fs testy understands.
///
/// Encoded as JSON in the prompt text, with a `command` tag. Example:
///
/// ```json
/// {"command":"write_file","path":"/scratch/out.md","content":"hello"}
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum FsTestyCommand {
    /// Emit an ACP `fs/write_text_file` request from the agent to the
    /// client with the given path and content. Responds with `"ok:<path>"`
    /// on success or `"error:..."` on failure.
    WriteFile { path: PathBuf, content: String },
    /// Emit an ACP `fs/read_text_file` request and respond with the
    /// returned content.
    ReadFile { path: PathBuf },
    /// Respond with `"ready"` — useful for liveness probes that don't
    /// want to trigger a real fs emission.
    Ready,
}

async fn process_prompt(
    request: PromptRequest,
    responder: Responder<PromptResponse>,
    connection: ConnectionTo<Client>,
) -> Result<(), sacp::Error> {
    let session_id = request.session_id.clone();
    let input_text = extract_text_from_prompt(&request.prompt);
    let command = serde_json::from_str::<FsTestyCommand>(&input_text).unwrap_or(
        FsTestyCommand::Ready,
    );

    let response_text = match command {
        FsTestyCommand::Ready => "ready".to_string(),
        FsTestyCommand::WriteFile { path, content } => {
            let write_request = WriteTextFileRequest::new(session_id.clone(), &path, content);
            match connection
                .send_request(write_request)
                .block_task()
                .await
            {
                Ok(_) => format!("ok:{}", path.display()),
                Err(error) => format!("error:{error}"),
            }
        }
        FsTestyCommand::ReadFile { path } => {
            let read_request = ReadTextFileRequest::new(session_id.clone(), &path);
            match connection
                .send_request(read_request)
                .block_task()
                .await
            {
                Ok(response) => response.content,
                Err(error) => format!("error:{error}"),
            }
        }
    };

    connection.send_notification(SessionNotification::new(
        session_id,
        SessionUpdate::AgentMessageChunk(ContentChunk::new(response_text.into())),
    ))?;

    responder.respond(PromptResponse::new(StopReason::EndTurn))
}

#[derive(Clone, Default)]
struct FsTesty;

impl ConnectTo<Client> for FsTesty {
    async fn connect_to(self, client: impl ConnectTo<Agent>) -> Result<(), sacp::Error> {
        Agent
            .builder()
            .name("fireline-testy-fs")
            .on_receive_request(
                async |initialize: InitializeRequest, responder, _cx| {
                    responder.respond(
                        InitializeResponse::new(initialize.protocol_version)
                            .agent_capabilities(AgentCapabilities::new()),
                    )
                },
                sacp::on_receive_request!(),
            )
            .on_receive_request(
                async move |_request: NewSessionRequest, responder, _cx| {
                    let session_id = SessionId::new(uuid::Uuid::new_v4().to_string());
                    responder.respond(NewSessionResponse::new(session_id))
                },
                sacp::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: PromptRequest, responder, cx| {
                    let cx_clone = cx.clone();
                    cx.spawn(async move { process_prompt(request, responder, cx_clone).await })
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

#[tokio::main]
async fn main() -> Result<()> {
    FsTesty::default()
        .connect_to(sacp_tokio::Stdio::new())
        .await?;
    Ok(())
}
