//! `/acp` WebSocket route handler.
//!
//! Each WebSocket connection gets a fresh conductor with a caller-provided
//! base component chain and a `DurableStreamTracer` attached via `trace_to`.
//! The writer observes ACP traffic and emits `STATE-PROTOCOL` entity changes
//! onto the runtime's durable state stream.
//!
//! This is the only place in the binary that knows about both axum and the
//! conductor — it's the explicit "developer wires HTTP into the substrate"
//! point that the rivet HTTP server pattern advocates.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{
    State,
    ws::{Message, WebSocket, WebSocketUpgrade},
};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use durable_streams::Producer;
use futures::{SinkExt, StreamExt};
use fireline_tools::peer::extract_remote_trace_context;
use sacp::{Agent, Client, Conductor, DynConnectTo, Lines};
use sacp_conductor::{ConductorImpl, McpBridgeMode, trace::WriteEvent};
use tracing::Instrument as _;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

use crate::{
    AgentPlaneTracer, TopologyRegistry, TopologySpec,
    shared_terminal::{AttachError, SharedTerminal, SharedTerminalAttachment},
    trace::{BoxedTraceWriter, CompositeTraceWriter, DurableStreamTracer},
};

pub type BaseComponentsFactory = Arc<dyn Fn() -> Vec<DynConnectTo<Conductor>> + Send + Sync>;

#[derive(Clone)]
pub struct AcpRouteState {
    pub conductor_name: String,
    pub host_key: String,
    pub node_id: String,
    pub host_id: String,
    pub state_producer: Producer,
    pub shared_terminal: SharedTerminal,
    pub topology_registry: TopologyRegistry,
    pub topology: TopologySpec,
    pub base_components_factory: BaseComponentsFactory,
}

pub fn router(state: AcpRouteState) -> Router {
    Router::new()
        .route("/acp", get(acp_websocket_handler))
        .with_state(state)
}

async fn acp_websocket_handler(
    State(app): State<AcpRouteState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let terminal_attachment = match attach_terminal_with_grace_period(&app.shared_terminal).await {
        Ok(attachment) => attachment,
        Err(AttachError::Busy) => {
            return (StatusCode::CONFLICT, "runtime_busy").into_response();
        }
        Err(AttachError::Closed) => {
            return (StatusCode::SERVICE_UNAVAILABLE, "runtime_closed").into_response();
        }
    };

    ws.on_upgrade(move |socket| async move {
        let connection_id = format!("conn:{}", Uuid::new_v4());
        let mut components = (app.base_components_factory)();
        let resolved_topology = match app.topology_registry.build(&app.topology) {
            Ok(resolved_topology) => resolved_topology,
            Err(error) => {
                tracing::warn!(error = %error, "failed to build host topology components");
                return;
            }
        };
        components.extend(resolved_topology.proxy_components);

        let mut trace_writers = Vec::with_capacity(1 + resolved_topology.trace_writers.len());
        trace_writers.push(Box::new(AgentPlaneTracer::new()) as BoxedTraceWriter);
        trace_writers.push(Box::new(DurableStreamTracer::new_with_host_context(
            app.state_producer.clone(),
            app.host_key.clone(),
            app.host_id.clone(),
            app.node_id.clone(),
            connection_id,
        )) as BoxedTraceWriter);
        trace_writers.extend(resolved_topology.trace_writers);
        let conductor = build_conductor_with_terminal(
            app.conductor_name.clone(),
            components,
            DynConnectTo::<Client>::new(terminal_attachment),
            CompositeTraceWriter::new(trace_writers),
        );

        if let Err(error) = handle_upgrade(conductor, socket).await {
            tracing::warn!(error = %error, "ACP session ended");
        }
    })
}

async fn attach_terminal_with_grace_period(
    shared_terminal: &SharedTerminal,
) -> Result<SharedTerminalAttachment, AttachError> {
    const BUSY_RETRY_ATTEMPTS: usize = 10;
    const BUSY_RETRY_DELAY: Duration = Duration::from_millis(10);

    let mut attempts = 0usize;
    loop {
        match shared_terminal.try_attach().await {
            Ok(attachment) => return Ok(attachment),
            // Sequential ACP helpers can reconnect before the previous
            // attachment's async detach signal has propagated through the
            // actor. Give that teardown path a brief grace period before
            // surfacing a real `runtime_busy` conflict.
            Err(AttachError::Busy) if attempts < BUSY_RETRY_ATTEMPTS => {
                attempts += 1;
                tokio::time::sleep(BUSY_RETRY_DELAY).await;
            }
            Err(error) => return Err(error),
        }
    }
}

fn build_conductor_with_terminal(
    name: impl ToString,
    components: Vec<DynConnectTo<Conductor>>,
    terminal: DynConnectTo<Client>,
    trace_writer: impl WriteEvent,
) -> ConductorImpl<Agent> {
    let mut components = Some(components);
    let mut terminal = Some(terminal);

    ConductorImpl::new_agent(
        name,
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

async fn handle_upgrade(conductor: ConductorImpl<Agent>, socket: WebSocket) -> anyhow::Result<()> {
    let debug = AcpDebug::from_env();
    debug.log_open();
    let (write, mut read) = socket.split();

    let outgoing_debug = debug.clone();
    let outgoing = SinkExt::with(
        SinkExt::sink_map_err(write, std::io::Error::other),
        move |line: String| {
            let debug = outgoing_debug.clone();
            async move {
                debug.log_send(&line);
                Ok::<_, std::io::Error>(Message::Text(line.into()))
            }
        },
    );

    let first_line = read_initial_line(&mut read, &debug).await?;
    let inbound_context = first_line
        .as_deref()
        .and_then(extract_remote_trace_context);

    let incoming_debug = debug.clone();
    let remaining_incoming = StreamExt::filter_map(read, move |message| {
        let debug = incoming_debug.clone();
        async move { stream_message_to_line(message, &debug) }
    });
    let incoming = futures::stream::once(async move {
        first_line.map(Ok::<_, std::io::Error>)
    })
    .filter_map(|item| async move { item })
    .chain(remaining_incoming);

    let serve_connection = async move {
        sacp::ConnectTo::<Agent>::connect_to(Lines::new(outgoing, incoming), conductor).await?;
        Ok::<_, sacp::Error>(())
    };

    let result = if let Some(parent_context) = inbound_context {
        let inbound_span = tracing::info_span!("fireline.peer_call.inbound");
        let _ = inbound_span.set_parent(parent_context);
        serve_connection.instrument(inbound_span).await
    } else {
        serve_connection.await
    };

    if let Err(error) = &result {
        debug.log_close_error(error);
    }

    result?;

    Ok(())
}

async fn read_initial_line(
    read: &mut futures::stream::SplitStream<WebSocket>,
    debug: &AcpDebug,
) -> std::io::Result<Option<String>> {
    while let Some(message) = read.next().await {
        if let Some(result) = stream_message_to_line(message, debug) {
            return result.map(Some);
        }
    }
    debug.log_close_without_frame();
    Ok(None)
}

fn stream_message_to_line(
    message: Result<Message, axum::Error>,
    debug: &AcpDebug,
) -> Option<Result<String, std::io::Error>> {
    match message {
        Ok(Message::Text(text)) => normalized_line(text.as_str(), debug),
        Ok(Message::Binary(bytes)) => String::from_utf8(bytes.to_vec())
            .ok()
            .and_then(|text| normalized_line(&text, debug)),
        Ok(Message::Close(frame)) => {
            debug.log_close_frame(frame.as_ref().map(|frame| (frame.code, frame.reason.as_str())));
            Some(Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "websocket closed",
            )))
        }
        Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => None,
        Err(err) => {
            debug.log_close_transport_error(&err);
            Some(Err(std::io::Error::other(err)))
        }
    }
}

#[derive(Clone, Default)]
struct AcpDebug {
    enabled: bool,
    last_message: Arc<Mutex<Option<String>>>,
}

impl AcpDebug {
    fn from_env() -> Self {
        Self {
            enabled: std::env::var("FIRELINE_ACP_DEBUG")
                .map(|value| value != "0" && !value.is_empty())
                .unwrap_or(false),
            last_message: Arc::new(Mutex::new(None)),
        }
    }

    fn log_open(&self) {
        if self.enabled {
            tracing::info!("FL-DEBUG[acp-open]");
            eprintln!("FL-DEBUG[acp-open]");
        }
    }

    fn log_recv(&self, line: &str) {
        if self.enabled {
            let msg = truncate_for_debug(line);
            tracing::info!("FL-DEBUG[acp-recv] {}", msg);
            eprintln!("FL-DEBUG[acp-recv] {}", msg);
        }
        self.set_last_message(line);
    }

    fn log_send(&self, line: &str) {
        if self.enabled {
            let msg = truncate_for_debug(line);
            tracing::info!("FL-DEBUG[acp-send] {}", msg);
            eprintln!("FL-DEBUG[acp-send] {}", msg);
        }
        self.set_last_message(line);
    }

    fn log_close_frame(&self, frame: Option<(u16, &str)>) {
        if !self.enabled {
            return;
        }
        if let Some((code, reason)) = frame {
            tracing::info!("FL-DEBUG[acp-close] code={} reason={}", code, reason);
            eprintln!("FL-DEBUG[acp-close] code={} reason={}", code, reason);
        } else {
            tracing::info!("FL-DEBUG[acp-close] code=none reason=");
            eprintln!("FL-DEBUG[acp-close] code=none reason=");
        }
        self.log_last_before_close();
    }

    fn log_close_transport_error(&self, err: &axum::Error) {
        if !self.enabled {
            return;
        }
        tracing::info!("FL-DEBUG[acp-close] code=1006 reason={}", err);
        eprintln!("FL-DEBUG[acp-close] code=1006 reason={}", err);
        self.log_last_before_close();
    }

    fn log_close_error(&self, err: &sacp::Error) {
        if !self.enabled {
            return;
        }
        tracing::info!("FL-DEBUG[acp-close] code=1006 reason={}", err);
        eprintln!("FL-DEBUG[acp-close] code=1006 reason={}", err);
        self.log_last_before_close();
    }

    fn log_close_without_frame(&self) {
        if !self.enabled {
            return;
        }
        tracing::info!("FL-DEBUG[acp-close] code=none reason=stream-ended");
        eprintln!("FL-DEBUG[acp-close] code=none reason=stream-ended");
        self.log_last_before_close();
    }

    fn log_last_before_close(&self) {
        let last = self
            .last_message
            .lock()
            .ok()
            .and_then(|last| last.clone())
            .unwrap_or_default();
        let msg = truncate_for_debug(&last);
        tracing::info!("FL-DEBUG[acp-last-before-close] {}", msg);
        eprintln!("FL-DEBUG[acp-last-before-close] {}", msg);
    }

    fn set_last_message(&self, line: &str) {
        if let Ok(mut slot) = self.last_message.lock() {
            *slot = Some(line.to_string());
        }
    }
}

fn normalized_line(raw: &str, debug: &AcpDebug) -> Option<Result<String, std::io::Error>> {
    let line = raw.trim().to_string();
    if line.is_empty() {
        return None;
    }
    debug.log_recv(&line);
    emit_request_span(&line);
    Some(Ok(line))
}

fn truncate_for_debug(line: &str) -> String {
    const LIMIT: usize = 200;
    if line.len() <= LIMIT {
        return line.to_string();
    }
    format!("{}…", &line[..LIMIT])
}

fn emit_request_span(line: &str) {
    let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    let Some(method) = message.get("method").and_then(serde_json::Value::as_str) else {
        return;
    };

    match method {
        "session/new" => {
            tracing::info_span!("fireline.session.new").in_scope(|| {});
        }
        "session/prompt" => {
            let session_id = message
                .get("params")
                .and_then(|params| params.get("sessionId"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            tracing::info_span!("fireline.session.prompt", session_id = %session_id)
                .in_scope(|| {});
        }
        _ => {}
    }
}
