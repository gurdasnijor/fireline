//! `/acp` WebSocket route handler.
//!
//! Each WebSocket connection gets a fresh conductor with the
//! `LoadCoordinatorComponent` and `PeerComponent` in its component chain and a
//! `DurableStreamTracer` attached via `trace_to`. The writer observes ACP
//! traffic and emits `STATE-PROTOCOL` entity changes onto the runtime's
//! durable state stream. The handler delegates to
//! `fireline_conductor::transports::websocket::handle_upgrade` for
//! the actual byte-stream wrapping and conductor execution.
//!
//! This is the only place in the binary that knows about both axum
//! and the conductor — it's the explicit "developer wires HTTP into
//! the substrate" point that the rivet HTTP server pattern advocates.

use axum::extract::{ws::WebSocketUpgrade, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use sacp::{Client, Conductor, DynConnectTo};
use std::time::Duration;
use uuid::Uuid;

use crate::{
    build,
    shared_terminal::{AttachError, SharedTerminal, SharedTerminalAttachment},
    trace::{CompositeTraceWriter, DurableStreamTracer},
    transports,
};

pub(crate) fn router(state: crate::bootstrap::AppState) -> Router {
    Router::new()
        .route("/acp", get(acp_websocket_handler))
        .with_state(state)
}

async fn acp_websocket_handler(
    State(app): State<crate::bootstrap::AppState>,
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
        let logical_connection_id = format!("conn:{}", Uuid::new_v4());
        let mut components: Vec<DynConnectTo<Conductor>> = vec![DynConnectTo::new(
            crate::load_coordinator::LoadCoordinatorComponent::new(app.session_index.clone()),
        )];
        let resolved_topology = match app.topology_registry.build(&app.topology) {
            Ok(resolved_topology) => resolved_topology,
            Err(error) => {
                tracing::warn!(error = %error, "failed to build runtime topology components");
                return;
            }
        };
        components.extend(resolved_topology.proxy_components);

        let mut trace_writers = Vec::with_capacity(1 + resolved_topology.trace_writers.len());
        trace_writers.push(Box::new(DurableStreamTracer::new_with_runtime_context(
            app.state_producer.clone(),
            app.runtime_key.clone(),
            app.runtime_id.clone(),
            app.node_id.clone(),
            logical_connection_id,
        )) as crate::trace::BoxedTraceWriter);
        trace_writers.extend(resolved_topology.trace_writers);
        let conductor = build::build_conductor_with_terminal(
            app.conductor_name.clone(),
            components,
            DynConnectTo::<Client>::new(terminal_attachment),
            CompositeTraceWriter::new(trace_writers),
        );

        if let Err(e) = transports::websocket::handle_upgrade(conductor, socket).await {
            tracing::warn!(error = %e, "ACP session ended");
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
