//! Peer wire transport.
//!
//! `prompt_peer` reaches the target peer by opening a normal ACP client
//! connection to the peer's hosted `/acp` endpoint, initializing, starting a
//! session, and sending a prompt. This stays on the SDK's normal live-session
//! path rather than introducing a second Fireline-specific peer protocol.

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use serde_json::{Map, Value};

use super::directory::Peer;

#[derive(Debug, Clone)]
pub(crate) struct PeerCallResult {
    pub child_session_id: String,
    pub response_text: String,
    pub stop_reason: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ParentLineage {
    pub trace_id: Option<String>,
    pub parent_prompt_turn_id: Option<String>,
}

struct WebSocketTransport {
    url: String,
}

impl sacp::ConnectTo<sacp::Client> for WebSocketTransport {
    async fn connect_to(
        self,
        client: impl sacp::ConnectTo<sacp::Agent>,
    ) -> Result<(), sacp::Error> {
        let (ws, _) = tokio_tungstenite::connect_async(self.url.as_str())
            .await
            .map_err(|e| sacp::util::internal_error(format!("WebSocket connect: {e}")))?;

        let (write, read) = StreamExt::split(ws);

        let outgoing = SinkExt::with(
            SinkExt::sink_map_err(write, std::io::Error::other),
            |line: String| async move {
                Ok::<_, std::io::Error>(tokio_tungstenite::tungstenite::Message::Text(line.into()))
            },
        );

        let incoming = StreamExt::filter_map(read, |msg| async move {
            match msg {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                    let line = text.trim().to_string();
                    if line.is_empty() {
                        None
                    } else {
                        Some(Ok(line))
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Binary(bytes)) => {
                    String::from_utf8(bytes.to_vec()).ok().and_then(|text| {
                        let line = text.trim().to_string();
                        if line.is_empty() {
                            None
                        } else {
                            Some(Ok(line))
                        }
                    })
                }
                Ok(_) => None,
                Err(err) => Some(Err(std::io::Error::other(err))),
            }
        });

        sacp::ConnectTo::<sacp::Client>::connect_to(sacp::Lines::new(outgoing, incoming), client)
            .await
    }
}

pub(crate) async fn dispatch_peer_call(
    peer: &Peer,
    prompt_text: &str,
    parent_lineage: Option<ParentLineage>,
) -> Result<PeerCallResult> {
    let transport = WebSocketTransport {
        url: peer.acp_url.clone(),
    };
    let prompt_text = prompt_text.to_string();
    let init_request = initialize_request(parent_lineage);

    sacp::Client
        .builder()
        .name(format!("fireline-peer-client-{}", peer.agent_name))
        .on_receive_request(
            async move |req: agent_client_protocol::RequestPermissionRequest, responder, _cx| {
                let outcome = if let Some(opt) = req.options.first() {
                    agent_client_protocol::RequestPermissionOutcome::Selected(
                        agent_client_protocol::SelectedPermissionOutcome::new(
                            opt.option_id.clone(),
                        ),
                    )
                } else {
                    agent_client_protocol::RequestPermissionOutcome::Cancelled
                };
                responder.respond(agent_client_protocol::RequestPermissionResponse::new(outcome))
            },
            sacp::on_receive_request!(),
        )
        .connect_with(transport, async move |cx| {
            cx.send_request(init_request).block_task().await?;

            cx.build_session(std::path::Path::new("."))
                .block_task()
                .run_until(async |mut session| {
                    let child_session_id = session.session_id().to_string();
                    session.send_prompt(&prompt_text)?;

                    let mut response_text = String::new();
                    loop {
                        let update = session.read_update().await?;
                        match update {
                            sacp::SessionMessage::SessionMessage(dispatch) => {
                                sacp::util::MatchDispatch::new(dispatch)
                                    .if_notification(async |notif: agent_client_protocol::SessionNotification| {
                                        if let agent_client_protocol::SessionUpdate::AgentMessageChunk(
                                            agent_client_protocol::ContentChunk {
                                                content: agent_client_protocol::ContentBlock::Text(text),
                                                ..
                                            },
                                        ) = notif.update
                                        {
                                            response_text.push_str(&text.text);
                                        }
                                        Ok(())
                                    })
                                    .await
                                    .otherwise_ignore()?;
                            }
                            sacp::SessionMessage::StopReason(reason) => {
                                break Ok(PeerCallResult {
                                    child_session_id: child_session_id.clone(),
                                    response_text,
                                    stop_reason: format!("{reason:?}"),
                                });
                            }
                            _ => {}
                        }
                    }
                })
                .await
        })
        .await
        .map_err(anyhow::Error::from)
        .with_context(|| format!("dispatch prompt to peer {}", peer.agent_name))
}

fn initialize_request(
    parent_lineage: Option<ParentLineage>,
) -> agent_client_protocol::InitializeRequest {
    let mut init = agent_client_protocol::InitializeRequest::new(
        agent_client_protocol::ProtocolVersion::LATEST,
    );

    let Some(lineage) = parent_lineage else {
        return init;
    };

    let mut fireline = Map::new();
    if let Some(trace_id) = lineage.trace_id {
        fireline.insert("traceId".to_string(), Value::String(trace_id));
    }
    if let Some(parent_prompt_turn_id) = lineage.parent_prompt_turn_id {
        fireline.insert(
            "parentPromptTurnId".to_string(),
            Value::String(parent_prompt_turn_id),
        );
    }

    if fireline.is_empty() {
        return init;
    }

    let mut meta = Map::new();
    meta.insert("fireline".to_string(), Value::Object(fireline));

    init = init.meta(meta);
    init
}
