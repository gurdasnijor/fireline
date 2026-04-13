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
    NewSessionResponse, PromptRequest, PromptResponse, SessionId, SessionNotification,
    SessionUpdate, StopReason, ToolCall, ToolCallId, ToolCallStatus, ToolCallUpdate,
    ToolCallUpdateFields,
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
        responder.respond(PromptResponse::new(StopReason::EndTurn))
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

fn build_conductor(
    components: Vec<DynConnectTo<Conductor>>,
    trace_writer: impl WriteEvent,
) -> ConductorImpl<Agent> {
    let mut components = Some(components);
    let mut terminal = Some(DynConnectTo::<Client>::new(ToolEchoAgent));
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

async fn wait_for_permission_request(stream_url: &str, session_id: &SessionId) -> Result<String> {
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
                let Some(request_id) = value.get("requestId") else {
                    continue;
                };
                return Ok(match request_id {
                    serde_json::Value::String(text) => text.clone(),
                    serde_json::Value::Number(number) => number.to_string(),
                    other => other.to_string(),
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
