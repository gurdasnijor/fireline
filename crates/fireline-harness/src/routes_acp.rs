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

use std::sync::Arc;
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
use sacp::{Agent, Client, Conductor, DynConnectTo, Lines};
use sacp_conductor::{ConductorImpl, McpBridgeMode, trace::WriteEvent};
use uuid::Uuid;

use crate::{
    TopologyRegistry, TopologySpec,
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
    let (write, read) = socket.split();

    let outgoing = SinkExt::with(
        SinkExt::sink_map_err(write, std::io::Error::other),
        |line: String| async move { Ok::<_, std::io::Error>(Message::Text(line.into())) },
    );

    let incoming = StreamExt::filter_map(read, |message| async move {
        match message {
            Ok(Message::Text(text)) => {
                let line = text.trim().to_string();
                if line.is_empty() {
                    None
                } else {
                    Some(Ok(line))
                }
            }
            Ok(Message::Binary(bytes)) => String::from_utf8(bytes.to_vec()).ok().and_then(|text| {
                let line = text.trim().to_string();
                if line.is_empty() {
                    None
                } else {
                    Some(Ok(line))
                }
            }),
            Ok(Message::Close(_)) => Some(Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "websocket closed",
            ))),
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => None,
            Err(err) => Some(Err(std::io::Error::other(err))),
        }
    });

    sacp::ConnectTo::<Agent>::connect_to(Lines::new(outgoing, incoming), conductor).await?;
    Ok(())
}
