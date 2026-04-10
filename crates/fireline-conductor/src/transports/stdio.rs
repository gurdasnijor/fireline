//! Stdio transport adapter.
//!
//! Wraps `tokio::io::stdin` / `tokio::io::stdout` in a
//! [`sacp::ByteStreams`] and runs the conductor over it. Used for
//! command-line invocation (`fireline --stdio`) and for any process
//! that wants to spawn the Fireline binary as a subprocess and talk
//! to it via pipes.

// TODO: implement handle_stdio
//
// Target signature:
//
// ```rust,ignore
// pub async fn handle_stdio(
//     conductor: sacp_conductor::ConductorImpl<sacp::Agent>,
// ) -> anyhow::Result<()>;
// ```
