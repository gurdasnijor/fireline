use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use durable_streams::{Client as DsClient, CreateOptions, Offset, Producer};
use fireline_harness::{
    AgentPlaneTracer, ApprovalAction, ApprovalConfig, ApprovalGateComponent, ApprovalMatch,
    ApprovalPolicy,
};
use sacp::schema::{
    AgentCapabilities, ContentChunk, InitializeRequest, InitializeResponse, NewSessionRequest,
    NewSessionResponse, PermissionOption, PermissionOptionId, PermissionOptionKind, PromptRequest,
    PromptResponse, RequestPermissionOutcome, RequestPermissionRequest, SessionId,
    SessionNotification, SessionUpdate, StopReason, ToolCall, ToolCallId, ToolCallStatus,
    ToolCallUpdate, ToolCallUpdateFields,
};
use sacp::{
    Agent, ByteStreams, Client, Conductor, ConnectTo, ConnectionTo, DynConnectTo, Responder,
};
use sacp_conductor::{ConductorImpl, McpBridgeMode, trace::WriteEvent};
use tokio::io::duplex;
use tokio::net::TcpListener;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

#[derive(Clone, Default)]
struct ToolEchoAgent;

impl ToolEchoAgent {
    async fn process_prompt(
        &self,
        request: PromptRequest,
        responder: Responder<PromptResponse>,
        connection: ConnectionTo<Client>,
    ) -> Result<(), sacp::Error> {
        let tool_call_id = ToolCallId::from("tool-demo-1".to_string());
        connection.send_notification(SessionNotification::new(
            request.session_id.clone(),
            SessionUpdate::ToolCall(ToolCall::new(tool_call_id.clone(), "demo.echo")),
        ))?;
        connection.send_notification(SessionNotification::new(
            request.session_id.clone(),
            SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                tool_call_id,
                ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
            )),
        ))?;
        connection.send_notification(SessionNotification::new(
            request.session_id,
            SessionUpdate::AgentMessageChunk(ContentChunk::new("tool complete".into())),
        ))?;
        responder.respond(PromptResponse::new(StopReason::EndTurn))?;
        Ok(())
    }
}

impl ConnectTo<Client> for ToolEchoAgent {
    async fn connect_to(self, client: impl ConnectTo<Agent>) -> Result<(), sacp::Error> {
        Agent
            .builder()
            .name("tool-echo-agent")
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
                    responder.respond(NewSessionResponse::new(SessionId::new(
                        uuid::Uuid::new_v4().to_string(),
                    )))
                },
                sacp::on_receive_request!(),
            )
            .on_receive_request(
                {
                    let agent = self.clone();
                    async move |request: PromptRequest, responder, cx| {
                        let cx_clone = cx.clone();
                        let agent = agent.clone();
                        cx.spawn(
                            async move { agent.process_prompt(request, responder, cx_clone).await },
                        )
                    }
                },
                sacp::on_receive_request!(),
            )
            .connect_to(client)
            .await
    }
}

#[derive(Clone, Default)]
struct ToolPermissionAgent;

impl ToolPermissionAgent {
    async fn process_prompt(
        &self,
        request: PromptRequest,
        responder: Responder<PromptResponse>,
        connection: ConnectionTo<Client>,
    ) -> Result<(), sacp::Error> {
        let prompt_text = request
            .prompt
            .iter()
            .filter_map(|block| match block {
                sacp::schema::ContentBlock::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");

        if !prompt_text.contains("use_tool") {
            connection.send_notification(SessionNotification::new(
                request.session_id,
                SessionUpdate::AgentMessageChunk(ContentChunk::new("plain response".into())),
            ))?;
            responder.respond(PromptResponse::new(StopReason::EndTurn))?;
            return Ok(());
        }

        let tool_call_id = ToolCallId::from("tool-demo-1".to_string());
        connection.send_notification(SessionNotification::new(
            request.session_id.clone(),
            SessionUpdate::ToolCall(ToolCall::new(tool_call_id.clone(), "demo.echo")),
        ))?;

        let permission = connection
            .send_request(RequestPermissionRequest::new(
                request.session_id.clone(),
                ToolCallUpdate::new(
                    tool_call_id.clone(),
                    ToolCallUpdateFields::new().title("demo.echo"),
                ),
                vec![
                    PermissionOption::new(
                        PermissionOptionId::new("allow-once"),
                        "Allow once",
                        PermissionOptionKind::AllowOnce,
                    ),
                    PermissionOption::new(
                        PermissionOptionId::new("reject-once"),
                        "Reject once",
                        PermissionOptionKind::RejectOnce,
                    ),
                ],
            ))
            .block_task()
            .await?;

        match permission.outcome {
            RequestPermissionOutcome::Selected(selected)
                if selected.option_id == PermissionOptionId::new("allow-once") =>
            {
                connection.send_notification(SessionNotification::new(
                    request.session_id.clone(),
                    SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                        tool_call_id,
                        ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
                    )),
                ))?;
                connection.send_notification(SessionNotification::new(
                    request.session_id,
                    SessionUpdate::AgentMessageChunk(ContentChunk::new("tool complete".into())),
                ))?;
                responder.respond(PromptResponse::new(StopReason::EndTurn))?;
            }
            _ => {
                connection.send_notification(SessionNotification::new(
                    request.session_id,
                    SessionUpdate::AgentMessageChunk(ContentChunk::new("tool blocked".into())),
                ))?;
                responder.respond(PromptResponse::new(StopReason::EndTurn))?;
            }
        }

        Ok(())
    }
}

impl ConnectTo<Client> for ToolPermissionAgent {
    async fn connect_to(self, client: impl ConnectTo<Agent>) -> Result<(), sacp::Error> {
        Agent
            .builder()
            .name("tool-permission-agent")
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
                    responder.respond(NewSessionResponse::new(SessionId::new(
                        uuid::Uuid::new_v4().to_string(),
                    )))
                },
                sacp::on_receive_request!(),
            )
            .on_receive_request(
                {
                    let agent = self.clone();
                    async move |request: PromptRequest, responder, cx| {
                        let cx_clone = cx.clone();
                        let agent = agent.clone();
                        cx.spawn(
                            async move { agent.process_prompt(request, responder, cx_clone).await },
                        )
                    }
                },
                sacp::on_receive_request!(),
            )
            .connect_to(client)
            .await
    }
}

fn build_conductor(
    components: Vec<DynConnectTo<Conductor>>,
    trace_writer: impl WriteEvent,
) -> ConductorImpl<Agent> {
    build_conductor_with_terminal(
        components,
        trace_writer,
        DynConnectTo::<Client>::new(ToolEchoAgent),
    )
}

fn build_conductor_with_terminal(
    components: Vec<DynConnectTo<Conductor>>,
    trace_writer: impl WriteEvent,
    terminal: DynConnectTo<Client>,
) -> ConductorImpl<Agent> {
    let mut components = Some(components);
    let mut terminal = Some(terminal);
    ConductorImpl::new_agent(
        "observability-test",
        move |req| async move {
            let components = components.take().ok_or_else(|| {
                sacp::util::internal_error("conductor components already instantiated")
            })?;
            let terminal = terminal
                .take()
                .ok_or_else(|| sacp::util::internal_error("terminal already instantiated"))?;
            Ok((req, components, terminal))
        },
        McpBridgeMode::default(),
    )
    .trace_to(trace_writer)
}

async fn handle_duplex(
    conductor: ConductorImpl<Agent>,
    stream: tokio::io::DuplexStream,
) -> Result<()> {
    let (read_half, write_half) = tokio::io::split(stream);
    conductor
        .run(ByteStreams::new(
            write_half.compat_write(),
            read_half.compat(),
        ))
        .await?;
    Ok(())
}

fn permission_producer(stream_url: &str) -> Producer {
    let client = DsClient::new();
    let mut stream = client.stream(stream_url);
    stream.set_content_type("application/json");
    stream
        .producer(format!("observability-test-{}", uuid::Uuid::new_v4()))
        .content_type("application/json")
        .build()
}

async fn ensure_json_stream_exists(stream_url: &str) -> Result<()> {
    let client = DsClient::new();
    let stream = client.stream(stream_url);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match stream
            .create_with(CreateOptions::new().content_type("application/json"))
            .await
        {
            Ok(_) | Err(durable_streams::StreamError::Conflict) => return Ok(()),
            Err(error) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                tracing::debug!(?error, stream_url, "retrying test stream creation");
            }
            Err(error) => {
                return Err(anyhow::Error::from(error))
                    .with_context(|| format!("create test stream '{stream_url}'"));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PermissionRequestEvent {
    request_id: Option<String>,
    tool_call_id: Option<String>,
}

async fn wait_for_permission_request_event(
    stream_url: &str,
    session_id: &SessionId,
) -> Result<PermissionRequestEvent> {
    let client = DsClient::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let stream = client.stream(stream_url);
        let mut reader = stream.read().offset(Offset::Beginning).build()?;
        while let Some(chunk) = reader.next_chunk().await? {
            if chunk.data.is_empty() {
                if chunk.up_to_date {
                    break;
                }
                continue;
            }
            let events: Vec<serde_json::Value> = serde_json::from_slice(&chunk.data)?;
            for event in events {
                let Some(value) = event.get("value") else {
                    continue;
                };
                if value.get("kind").and_then(serde_json::Value::as_str)
                    != Some("permission_request")
                {
                    continue;
                }
                if value.get("sessionId").and_then(serde_json::Value::as_str)
                    != Some(session_id.to_string().as_str())
                {
                    continue;
                }
                let request_id = value.get("requestId").map(|request_id| match request_id {
                    serde_json::Value::String(text) => text.clone(),
                    serde_json::Value::Number(number) => number.to_string(),
                    other => other.to_string(),
                });
                let tool_call_id = value
                    .get("toolCallId")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                return Ok(PermissionRequestEvent {
                    request_id,
                    tool_call_id,
                });
            }
            if chunk.up_to_date {
                break;
            }
        }
        if Instant::now() >= deadline {
            anyhow::bail!("permission_request did not appear on the state stream in time");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_permission_request(stream_url: &str, session_id: &SessionId) -> Result<String> {
    wait_for_permission_request_event(stream_url, session_id)
        .await?
        .request_id
        .context("permission_request did not carry requestId")
}

async fn append_approval_resolved(
    producer: &Producer,
    session_id: &SessionId,
    request_id: &str,
) -> Result<()> {
    producer.append_json(&serde_json::json!({
        "type": "permission",
        "key": format!("{session_id}:{request_id}:resolved"),
        "headers": { "operation": "insert" },
        "value": {
            "kind": "approval_resolved",
            "sessionId": session_id,
            "requestId": request_id,
            "allow": true,
            "resolvedBy": "observability-test",
            "createdAtMs": 1_700_000_000_000i64,
        }
    }));
    producer.flush().await?;
    Ok(())
}

async fn append_tool_approval_resolved(
    producer: &Producer,
    session_id: &SessionId,
    tool_call_id: &str,
) -> Result<()> {
    producer.append_json(&serde_json::json!({
        "type": "permission",
        "key": format!("{session_id}:{tool_call_id}:resolved"),
        "headers": { "operation": "insert" },
        "value": {
            "kind": "approval_resolved",
            "sessionId": session_id,
            "toolCallId": tool_call_id,
            "allow": true,
            "resolvedBy": "observability-test",
            "createdAtMs": 1_700_000_000_000i64,
        }
    }));
    producer.flush().await?;
    Ok(())
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[tokio::test(flavor = "current_thread")]
async fn agent_plane_observability_chain_runs_without_otlp_exporter() -> Result<()> {
    let _guard = tracing::subscriber::set_default(tracing_subscriber::registry());

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let stream_server = tokio::spawn(async move {
        axum::serve(listener, fireline_session::build_stream_router(None)?)
            .await
            .map_err(anyhow::Error::from)
    });

    let stream_name = format!("observability-agent-plane-{}", uuid::Uuid::new_v4());
    let stream_url = format!("http://{addr}/v1/stream/{stream_name}");
    ensure_json_stream_exists(&stream_url).await?;

    let producer = permission_producer(&stream_url);
    let approval_gate = ApprovalGateComponent::with_stream_and_timeout(
        ApprovalConfig {
            policies: vec![ApprovalPolicy {
                match_rule: ApprovalMatch::PromptContains {
                    needle: "pause_here".to_string(),
                },
                action: ApprovalAction::RequireApproval,
                reason: "demo approval gate".to_string(),
            }],
        },
        Some(stream_url.clone()),
        Some(producer.clone()),
        Some(Duration::from_secs(10)),
    );

    let conductor = build_conductor(
        vec![DynConnectTo::new(approval_gate)],
        AgentPlaneTracer::new(),
    );
    let (client_stream, conductor_stream) = duplex(16 * 1024);
    let conductor_task =
        tokio::spawn(async move { handle_duplex(conductor, conductor_stream).await });
    let (client_read, client_write) = tokio::io::split(client_stream);

    let approval_stream_url = stream_url.clone();
    let approval_producer = producer.clone();
    let client_result = sacp::Client
        .builder()
        .name("observability-test-client")
        .connect_with(
            ByteStreams::new(client_write.compat_write(), client_read.compat()),
            async move |cx| {
                cx.send_request(InitializeRequest::new(
                    sacp::schema::ProtocolVersion::LATEST,
                ))
                .block_task()
                .await?;

                let cwd = repo_root();
                let response: String = cx
                    .build_session(cwd)
                    .block_task()
                    .run_until(async move |mut session| -> Result<String, sacp::Error> {
                        let session_id = session.session_id().clone();
                        let approval_task = tokio::spawn(async move {
                            let request_id =
                                wait_for_permission_request(&approval_stream_url, &session_id)
                                    .await
                                    .map_err(|error| error.to_string())?;
                            append_approval_resolved(&approval_producer, &session_id, &request_id)
                                .await
                                .map_err(|error| error.to_string())
                        });

                        session.send_prompt("please pause_here and emit a tool call")?;
                        let output = session.read_to_string().await?;
                        let approval_result = approval_task.await.map_err(|error| {
                            sacp::util::internal_error(format!("approval task panicked: {error}"))
                        })?;
                        approval_result.map_err(sacp::util::internal_error)?;
                        Ok(output)
                    })
                    .await?;

                if !response.contains("tool complete") {
                    return Err(sacp::util::internal_error(format!(
                        "expected tool completion output, got: {response}"
                    )));
                }
                Ok(())
            },
        )
        .await;

    conductor_task.abort();
    stream_server.abort();

    client_result.context("client/session flow failed")?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn tool_call_scoped_approval_only_blocks_real_tool_calls() -> Result<()> {
    let _guard = tracing::subscriber::set_default(tracing_subscriber::registry());

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let stream_server = tokio::spawn(async move {
        axum::serve(listener, fireline_session::build_stream_router(None)?)
            .await
            .map_err(anyhow::Error::from)
    });

    let stream_name = format!("approval-tool-scope-{}", uuid::Uuid::new_v4());
    let stream_url = format!("http://{addr}/v1/stream/{stream_name}");
    ensure_json_stream_exists(&stream_url).await?;

    let producer = permission_producer(&stream_url);
    let approval_gate = ApprovalGateComponent::with_stream_and_timeout(
        ApprovalConfig {
            policies: vec![ApprovalPolicy {
                match_rule: ApprovalMatch::ToolPrefix {
                    prefix: "".to_string(),
                },
                action: ApprovalAction::RequireApproval,
                reason: "tool approval".to_string(),
            }],
        },
        Some(stream_url.clone()),
        Some(producer.clone()),
        Some(Duration::from_secs(10)),
    );

    let conductor = build_conductor_with_terminal(
        vec![DynConnectTo::new(approval_gate)],
        AgentPlaneTracer::new(),
        DynConnectTo::<Client>::new(ToolPermissionAgent),
    );
    let (client_stream, conductor_stream) = duplex(16 * 1024);
    let conductor_task =
        tokio::spawn(async move { handle_duplex(conductor, conductor_stream).await });
    let (client_read, client_write) = tokio::io::split(client_stream);

    let approval_stream_url = stream_url.clone();
    let approval_producer = producer.clone();
    let client_result = sacp::Client
        .builder()
        .name("tool-scope-test-client")
        .connect_with(
            ByteStreams::new(client_write.compat_write(), client_read.compat()),
            async move |cx| {
                cx.send_request(InitializeRequest::new(
                    sacp::schema::ProtocolVersion::LATEST,
                ))
                .block_task()
                .await?;

                let cwd = repo_root();
                cx.build_session(cwd)
                    .block_task()
                    .run_until(async move |mut session| -> Result<(), sacp::Error> {
                        let session_id = session.session_id().clone();

                        session.send_prompt("plain prompt without approval")?;
                        let plain_output = tokio::time::timeout(
                            Duration::from_secs(5),
                            session.read_to_string(),
                        )
                        .await
                        .map_err(|_| {
                            sacp::util::internal_error(
                                "plain prompt did not complete within timeout",
                            )
                        })??;
                        if plain_output != "plain response" {
                            return Err(sacp::util::internal_error(format!(
                                "expected plain response without approval gating, got: {plain_output}"
                            )));
                        }

                        let permission_before = count_permission_events(&approval_stream_url).await?;
                        let tool_session_id = session_id.clone();
                        let approval_stream_url_for_task = approval_stream_url.clone();
                        let approval_producer_for_task = approval_producer.clone();
                        let approval_task = tokio::spawn(async move {
                            let event =
                                wait_for_permission_request_event(
                                    &approval_stream_url_for_task,
                                    &tool_session_id,
                                )
                                .await
                                .map_err(|error| error.to_string())?;
                            let Some(tool_call_id) = event.tool_call_id else {
                                return Err("permission_request missing toolCallId".to_string());
                            };
                            if event.request_id.is_some() {
                                return Err(
                                    "tool-scoped permission_request unexpectedly carried requestId"
                                        .to_string(),
                                );
                            }
                            append_tool_approval_resolved(
                                &approval_producer_for_task,
                                &tool_session_id,
                                &tool_call_id,
                            )
                            .await
                            .map_err(|error| error.to_string())
                        });

                        session.send_prompt("please use_tool now")?;
                        let tool_output = tokio::time::timeout(
                            Duration::from_secs(15),
                            session.read_to_string(),
                        )
                        .await
                        .map_err(|_| {
                            sacp::util::internal_error(
                                "tool-scoped prompt did not complete within timeout",
                            )
                        })??;
                        if !tool_output.contains("tool complete") {
                            return Err(sacp::util::internal_error(format!(
                                "expected tool completion output after approval, got: {tool_output}"
                            )));
                        }

                        let approval_result = tokio::time::timeout(Duration::from_secs(15), approval_task)
                            .await
                            .map_err(|_| {
                                sacp::util::internal_error(
                                    "approval task did not resolve within timeout",
                                )
                            })?
                            .map_err(|error| {
                                sacp::util::internal_error(format!(
                                    "approval task panicked: {error}"
                                ))
                            })?;
                        approval_result.map_err(sacp::util::internal_error)?;

                        let permission_after = count_permission_events(&approval_stream_url).await?;
                        if permission_after != permission_before + 2 {
                            return Err(sacp::util::internal_error(format!(
                                "expected exactly one permission_request and one approval_resolved for the tool call, saw permission envelope delta {}",
                                permission_after - permission_before
                            )));
                        }

                        Ok(())
                    })
                    .await?;

                Ok(())
            },
        )
        .await;

    conductor_task.abort();
    stream_server.abort();

    client_result.context("tool-scoped approval flow failed")?;
    Ok(())
}

async fn count_permission_events(stream_url: &str) -> Result<usize> {
    let client = DsClient::new();
    let stream = client.stream(stream_url);
    let mut reader = stream.read().offset(Offset::Beginning).build()?;
    let mut count = 0usize;
    while let Some(chunk) = reader.next_chunk().await? {
        if !chunk.data.is_empty() {
            let events: Vec<serde_json::Value> = serde_json::from_slice(&chunk.data)?;
            count += events
                .into_iter()
                .filter(|event| {
                    event.get("type").and_then(serde_json::Value::as_str) == Some("permission")
                })
                .count();
        }
        if chunk.up_to_date {
            break;
        }
    }
    Ok(count)
}
