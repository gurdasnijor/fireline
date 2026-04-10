//! WebSocket transport adapter.
//!
//! Wraps an [`axum::extract::ws::WebSocket`] in a [`sacp::ByteStreams`]
//! and runs the conductor over it. Used by the binary's `/acp` route
//! handler — when a browser client opens a WebSocket to `/acp`, the
//! handler builds a fresh conductor and hands it to this adapter.
//!
//! Each WebSocket connection gets its own conductor instance with its
//! own component chain. The conductor's lifetime is bounded by the
//! WebSocket's lifetime.

// TODO: implement handle_upgrade
//
// Target signature:
//
// ```rust,ignore
// pub async fn handle_upgrade(
//     conductor: sacp_conductor::ConductorImpl<sacp::Agent>,
//     socket: axum::extract::ws::WebSocket,
// ) -> anyhow::Result<()>;
// ```
//
// Implementation sketch: bridge the WebSocket message stream to a
// `tokio::io::DuplexStream` via a small pump task, wrap the duplex
// in a `sacp::ByteStreams`, and call `conductor.run(byte_streams)`.
// On WebSocket close, abort the pump and the conductor unwinds.
