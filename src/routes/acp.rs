//! `/acp` WebSocket route handler.
//!
//! Each WebSocket connection gets a fresh conductor with the
//! `PeerComponent` in its component chain and a `DurableStreamTracer`
//! attached via `trace_to`. The writer observes ACP traffic and emits
//! `STATE-PROTOCOL` entity changes onto the runtime's durable state
//! stream. The handler delegates to
//! `fireline_conductor::transports::websocket::handle_upgrade` for
//! the actual byte-stream wrapping and conductor execution.
//!
//! This is the only place in the binary that knows about both axum
//! and the conductor — it's the explicit "developer wires HTTP into
//! the substrate" point that the rivet HTTP server pattern advocates.

use axum::Router;
use axum::extract::{State, ws::WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use fireline_conductor::{build, trace::DurableStreamTracer, transports};
use fireline_peer::PeerComponent;
use sacp::{Conductor, DynConnectTo};
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
    ws.on_upgrade(move |socket| async move {
        let logical_connection_id = format!("conn:{}", Uuid::new_v4());
        let components: Vec<DynConnectTo<Conductor>> = vec![DynConnectTo::new(PeerComponent::new(
            app.peer_directory_path.clone(),
        ))];

        let trace_writer = DurableStreamTracer::new(
            app.state_producer.clone(),
            app.runtime_id.clone(),
            logical_connection_id,
        );
        let conductor = build::build_subprocess_conductor(
            app.conductor_name.clone(),
            app.agent_command.clone(),
            components,
            trace_writer,
        );

        if let Err(e) = transports::websocket::handle_upgrade(conductor, socket).await {
            tracing::warn!(error = %e, "ACP session ended");
        }
    })
}
