use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use agent_client_protocol_test::testy::TestyCommand;
use anyhow::Result;
use durable_streams::{Client as DsClient, Offset};
use fireline_harness::TopologySpec;
use fireline_host::bootstrap::{BootstrapConfig, start};
use futures::{SinkExt, StreamExt};
use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter};
use serde_json::Value;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use uuid::Uuid;

#[path = "support/stream_server.rs"]
mod stream_server;

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

fn testy_bin() -> String {
    PathBuf::from(env!("CARGO_BIN_EXE_fireline-testy"))
        .display()
        .to_string()
}

fn temp_peer_directory() -> PathBuf {
    std::env::temp_dir().join(format!("fireline-peers-{}.toml", Uuid::new_v4()))
}

#[derive(Debug, Clone, Default)]
struct TestSpanExporter {
    spans: Arc<Mutex<Vec<SpanData>>>,
}

impl SpanExporter for TestSpanExporter {
    fn export(&self, batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
        let spans = self.spans.clone();
        async move {
            spans.lock().unwrap().extend(batch);
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Default)]
struct TelemetryCapture {
    spans: Arc<Mutex<Vec<SpanData>>>,
}

impl TelemetryCapture {
    fn clear(&self) {
        self.spans.lock().unwrap().clear();
    }

    fn snapshot(&self) -> Vec<SpanData> {
        self.spans.lock().unwrap().clone()
    }
}

fn telemetry_capture() -> &'static TelemetryCapture {
    static CAPTURE: OnceLock<TelemetryCapture> = OnceLock::new();
    static INIT: OnceLock<()> = OnceLock::new();

    let capture = CAPTURE.get_or_init(TelemetryCapture::default);
    INIT.get_or_init(|| {
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(TestSpanExporter {
                spans: capture.spans.clone(),
            })
            .build();
        let tracer = tracer_provider.tracer("fireline-mesh-baseline");
        global::set_tracer_provider(tracer_provider);
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_test_writer().without_time())
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .try_init()
            .expect("initialize mesh baseline tracing");
    });
    capture
}

#[tokio::test]
async fn mesh_baseline_exposes_peer_tools_and_prompts_remote_peer_over_acp() -> Result<()> {
    let peer_directory_path = temp_peer_directory();
    let stream_server = stream_server::TestStreamServer::spawn().await?;

    let handle_b = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "agent-b".to_string(),
        host_key: format!("runtime:{}", Uuid::new_v4()),
        node_id: "node:test-mesh".to_string(),
        agent_command: vec![testy_bin()],
        mounted_resources: Vec::new(),
        state_stream: None,
        durable_streams_url: stream_server.base_url.clone(),
        peer_directory_path: peer_directory_path.clone(),
        control_plane_url: None,
        topology: TopologySpec::default(),
    })
    .await?;

    let handle_a = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "agent-a".to_string(),
        host_key: format!("runtime:{}", Uuid::new_v4()),
        node_id: "node:test-mesh".to_string(),
        agent_command: vec![testy_bin()],
        mounted_resources: Vec::new(),
        state_stream: None,
        durable_streams_url: stream_server.base_url.clone(),
        peer_directory_path: peer_directory_path.clone(),
        control_plane_url: None,
        topology: TopologySpec::default(),
    })
    .await?;

    let tools = yopo::prompt(
        WebSocketTransport {
            url: handle_a.acp_url.clone(),
        },
        TestyCommand::ListTools {
            server: "fireline-peer".to_string(),
        }
        .to_prompt(),
    )
    .await?;

    assert!(
        tools.contains("list_peers") && tools.contains("prompt_peer"),
        "peer MCP server should expose list_peers and prompt_peer: {tools}"
    );

    let peers = yopo::prompt(
        WebSocketTransport {
            url: handle_a.acp_url.clone(),
        },
        TestyCommand::CallTool {
            server: "fireline-peer".to_string(),
            tool: "list_peers".to_string(),
            params: serde_json::json!({}),
        }
        .to_prompt(),
    )
    .await?;

    assert!(
        peers.contains("agent-a") && peers.contains("agent-b"),
        "list_peers should return both local runtimes: {peers}"
    );

    let prompt_peer = yopo::prompt(
        WebSocketTransport {
            url: handle_a.acp_url.clone(),
        },
        TestyCommand::CallTool {
            server: "fireline-peer".to_string(),
            tool: "prompt_peer".to_string(),
            params: serde_json::json!({
                "agentName": "agent-b",
                "prompt": TestyCommand::Echo {
                    message: "hello across mesh".to_string(),
                }
                .to_prompt(),
            }),
        }
        .to_prompt(),
    )
    .await?;

    assert!(
        prompt_peer.contains("agent-b") && prompt_peer.contains("hello across mesh"),
        "prompt_peer should return the remote peer response: {prompt_peer}"
    );

    let client = DsClient::new();
    let stream_a = client.stream(&handle_a.state_stream_url);
    let stream_b = client.stream(&handle_b.state_stream_url);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut body_a = String::new();
    let mut body_b = String::new();

    loop {
        body_a.clear();
        body_b.clear();

        let mut reader_a = stream_a.read().offset(Offset::Beginning).build()?;
        while let Some(chunk) = reader_a.next_chunk().await? {
            body_a.push_str(std::str::from_utf8(&chunk.data)?);
            if chunk.up_to_date {
                break;
            }
        }

        let mut reader_b = stream_b.read().offset(Offset::Beginning).build()?;
        while let Some(chunk) = reader_b.next_chunk().await? {
            body_b.push_str(std::str::from_utf8(&chunk.data)?);
            if chunk.up_to_date {
                break;
            }
        }

        let parent = find_prompt_request(&body_a, |text| text.contains("\"tool\":\"prompt_peer\""));
        let child = find_prompt_request(&body_b, |text| text.contains("hello across mesh"));

        if parent.is_some() && child.is_some() {
            break;
        }

        if tokio::time::Instant::now() >= deadline {
            break;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert!(
        body_b.contains("\"type\":\"prompt_request\""),
        "remote peer runtime should record the cross-runtime prompt as a prompt_request: {body_b}"
    );
    assert!(
        find_prompt_request(&body_a, |text| text.contains("\"tool\":\"prompt_peer\"")).is_some(),
        "missing parent prompt_request in runtime A: {body_a}"
    );
    assert!(
        find_prompt_request(&body_b, |text| text.contains("hello across mesh")).is_some(),
        "missing child prompt_request in runtime B: {body_b}"
    );
    assert!(
        !body_a.contains("\"type\":\"child_session_edge\""),
        "Phase 3 deletes child_session_edge emission: {body_a}"
    );

    handle_a.shutdown().await?;
    handle_b.shutdown().await?;
    stream_server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn peer_trace_context_propagates_across_prompt_peer() -> Result<()> {
    let telemetry = telemetry_capture();
    telemetry.clear();

    let peer_directory_path = temp_peer_directory();
    let stream_server = stream_server::TestStreamServer::spawn().await?;

    let handle_b = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "agent-b".to_string(),
        host_key: format!("runtime:{}", Uuid::new_v4()),
        node_id: "node:test-mesh".to_string(),
        agent_command: vec![testy_bin()],
        mounted_resources: Vec::new(),
        state_stream: None,
        durable_streams_url: stream_server.base_url.clone(),
        peer_directory_path: peer_directory_path.clone(),
        control_plane_url: None,
        topology: TopologySpec::default(),
    })
    .await?;

    let handle_a = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "agent-a".to_string(),
        host_key: format!("runtime:{}", Uuid::new_v4()),
        node_id: "node:test-mesh".to_string(),
        agent_command: vec![testy_bin()],
        mounted_resources: Vec::new(),
        state_stream: None,
        durable_streams_url: stream_server.base_url.clone(),
        peer_directory_path: peer_directory_path.clone(),
        control_plane_url: None,
        topology: TopologySpec::default(),
    })
    .await?;

    let _ = yopo::prompt(
        WebSocketTransport {
            url: handle_a.acp_url.clone(),
        },
        TestyCommand::CallTool {
            server: "fireline-peer".to_string(),
            tool: "prompt_peer".to_string(),
            params: serde_json::json!({
                "agentName": "agent-b",
                "prompt": TestyCommand::Echo {
                    message: "trace propagation".to_string(),
                }
                .to_prompt(),
            }),
        }
        .to_prompt(),
    )
    .await?;

    let client = DsClient::new();
    let stream_a = client.stream(&handle_a.state_stream_url);
    let stream_b = client.stream(&handle_b.state_stream_url);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let (parent_prompt, child_prompt) = loop {
        let body_a = read_stream_body(&stream_a).await?;
        let body_b = read_stream_body(&stream_b).await?;

        let parent =
            find_prompt_request(&body_a, |text| text.contains("\"tool\":\"prompt_peer\""));
        let child = find_prompt_request(&body_b, |text| text.contains("trace propagation"));

        if let (Some(parent), Some(child)) = (parent, child) {
            break (parent, child);
        }

        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for parent and child prompt_request rows");
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    let spans = wait_for_spans(telemetry, Duration::from_secs(5), |spans| {
        let outbound = find_named_span(spans, "fireline.peer_call.outbound");
        let inbound = find_named_span(spans, "fireline.peer_call.inbound");
        let child_prompt_span = spans.iter().find(|span| {
            span.name == "fireline.session.prompt"
                && span_attribute_eq(span, "session_id", &child_prompt.session_id)
        });
        outbound.is_some() && inbound.is_some() && child_prompt_span.is_some()
    })
    .await?;

    let outbound = find_named_span(&spans, "fireline.peer_call.outbound")
        .expect("missing fireline.peer_call.outbound span");
    let inbound = find_named_span(&spans, "fireline.peer_call.inbound")
        .expect("missing fireline.peer_call.inbound span");
    let child_prompt_span = spans
        .iter()
        .find(|span| {
            span.name == "fireline.session.prompt"
                && span_attribute_eq(span, "session_id", &child_prompt.session_id)
        })
        .expect("missing child fireline.session.prompt span");

    assert_eq!(
        inbound.span_context.trace_id(),
        outbound.span_context.trace_id(),
        "peer inbound span should inherit the outbound peer call trace id"
    );
    assert_eq!(
        inbound.parent_span_id,
        outbound.span_context.span_id(),
        "peer inbound span should point at the outbound peer-call span"
    );
    assert!(
        inbound.parent_span_is_remote,
        "peer inbound span should record a remote parent"
    );
    assert_eq!(
        child_prompt_span.span_context.trace_id(),
        outbound.span_context.trace_id(),
        "child session/prompt span should stay on the propagated trace"
    );
    assert_eq!(
        child_prompt_span.parent_span_id,
        inbound.span_context.span_id(),
        "child session/prompt span should hang off the inbound peer span"
    );
    assert_ne!(
        parent_prompt.request_id, child_prompt.request_id,
        "cross-host prompt_peer should still allocate a fresh child request id"
    );

    handle_a.shutdown().await?;
    handle_b.shutdown().await?;
    stream_server.shutdown().await;
    Ok(())
}

#[derive(Debug)]
struct PromptRequestEvent {
    request_id: String,
    session_id: String,
}

fn find_prompt_request(body: &str, predicate: impl Fn(&str) -> bool) -> Option<PromptRequestEvent> {
    parse_state_events(body).into_iter().find_map(|event| {
        if event.get("type")?.as_str()? != "prompt_request" {
            return None;
        }

        let value = event.get("value")?;
        let text = value.get("text").and_then(Value::as_str).unwrap_or("");
        if !predicate(text) {
            return None;
        }

        Some(PromptRequestEvent {
            request_id: value.get("requestId")?.as_str()?.to_string(),
            session_id: value.get("sessionId")?.as_str()?.to_string(),
        })
    })
}

fn parse_state_events(body: &str) -> Vec<Value> {
    match serde_json::from_str::<Value>(body) {
        Ok(Value::Array(events)) => events,
        Ok(value) => vec![value],
        Err(_) => {
            let mut stream = serde_json::Deserializer::from_str(body).into_iter::<Value>();
            std::iter::from_fn(move || stream.next())
                .filter_map(|result| result.ok())
                .collect()
        }
    }
}

async fn read_stream_body(stream: &durable_streams::DurableStream) -> Result<String> {
    let mut reader = stream.read().offset(Offset::Beginning).build()?;
    let mut body = String::new();
    while let Some(chunk) = reader.next_chunk().await? {
        body.push_str(std::str::from_utf8(&chunk.data)?);
        if chunk.up_to_date {
            break;
        }
    }
    Ok(body)
}

async fn wait_for_spans(
    telemetry: &TelemetryCapture,
    timeout: Duration,
    predicate: impl Fn(&[SpanData]) -> bool,
) -> Result<Vec<SpanData>> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let spans = telemetry.snapshot();
        if predicate(&spans) {
            return Ok(spans);
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for expected trace spans");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn find_named_span<'a>(spans: &'a [SpanData], name: &str) -> Option<&'a SpanData> {
    spans.iter().find(|span| span.name == name)
}

fn span_attribute_eq(span: &SpanData, key: &str, expected: &str) -> bool {
    span.attributes.iter().any(|attribute| {
        attribute.key.as_str() == key && attribute.value.to_string() == expected
    })
}
