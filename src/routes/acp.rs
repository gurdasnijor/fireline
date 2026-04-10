//! `/acp` WebSocket route handler.
//!
//! Each WebSocket connection gets a fresh conductor with the
//! `PeerComponent` in its component chain and a `DurableStreamTracer`
//! attached via `trace_to`. The handler delegates to
//! `fireline_conductor::transports::websocket::handle_upgrade` for
//! the actual byte-stream wrapping and conductor execution.
//!
//! This is the only place in the binary that knows about both axum
//! and the conductor — it's the explicit "developer wires HTTP into
//! the substrate" point that the rivet HTTP server pattern advocates.

// TODO: implement acp_websocket_handler
//
// Target shape:
//
// ```rust,ignore
// use axum::extract::{State, ws::WebSocketUpgrade};
// use axum::response::IntoResponse;
// use fireline_conductor::{build, transports, trace::DurableStreamTracer};
// use fireline_peer::PeerComponent;
// use sacp::{DynComponent, ProxyToConductor};
//
// pub async fn acp_websocket_handler(
//     State(app): State<crate::bootstrap::AppState>,
//     ws: WebSocketUpgrade,
// ) -> impl IntoResponse {
//     ws.on_upgrade(move |socket| async move {
//         let components: Vec<DynComponent<ProxyToConductor>> = vec![
//             DynComponent::new(PeerComponent::new(app.peer_directory_path.clone())),
//         ];
//         let trace_writer = DurableStreamTracer::new(
//             app.producer.clone(),
//             app.runtime_id.clone(),
//         );
//         let conductor = build::build_subprocess_conductor(
//             "fireline",
//             app.agent_command.clone(),
//             components,
//             trace_writer,
//         );
//         if let Err(e) = transports::websocket::handle_upgrade(conductor, socket).await {
//             tracing::warn!(error = %e, "ACP session ended");
//         }
//     })
// }
// ```
