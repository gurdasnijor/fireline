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

use axum::Router;
use axum::extract::{State, ws::WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use fireline_conductor::{build, lineage::LineageTracker, trace::DurableStreamTracer, transports};
use fireline_peer::PeerComponent;
use sacp::{Client, Conductor, DynConnectTo};
use uuid::Uuid;

pub fn router(state: crate::bootstrap::AppState) -> Router {
    Router::new()
        .route("/acp", get(acp_websocket_handler))
        .with_state(state)
}

pub async fn acp_websocket_handler(
    State(app): State<crate::bootstrap::AppState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let terminal_attachment = match app.shared_terminal.try_attach().await {
        Ok(attachment) => attachment,
        Err(fireline_conductor::shared_terminal::AttachError::Busy) => {
            return (StatusCode::CONFLICT, "runtime_busy").into_response();
        }
        Err(fireline_conductor::shared_terminal::AttachError::Closed) => {
            return (StatusCode::SERVICE_UNAVAILABLE, "runtime_closed").into_response();
        }
    };

    ws.on_upgrade(move |socket| async move {
        let logical_connection_id = format!("conn:{}", Uuid::new_v4());
        let lineage_tracker = LineageTracker::default();
        let components: Vec<DynConnectTo<Conductor>> = vec![
            DynConnectTo::new(crate::load_coordinator::LoadCoordinatorComponent::new(
                app.session_index.clone(),
            )),
            DynConnectTo::new(PeerComponent::new(
                app.peer_directory_path.clone(),
                lineage_tracker.clone(),
            )),
        ];

        let trace_writer = DurableStreamTracer::new_with_runtime_context(
            app.state_producer.clone(),
            app.runtime_key.clone(),
            app.runtime_id.clone(),
            app.node_id.clone(),
            logical_connection_id,
            lineage_tracker,
        );
        let conductor = build::build_conductor_with_terminal(
            app.conductor_name.clone(),
            components,
            DynConnectTo::<Client>::new(terminal_attachment),
            trace_writer,
        );

        if let Err(e) = transports::websocket::handle_upgrade(conductor, socket).await {
            tracing::warn!(error = %e, "ACP session ended");
        }
    })
}
