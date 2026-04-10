//! In-memory duplex transport adapter.
//!
//! Wraps a [`tokio::io::DuplexStream`] in a [`sacp::ByteStreams`] and
//! runs the conductor over it. Used in tests and for any in-process
//! integration where one side of the duplex is the conductor and the
//! other side is a test client.

// TODO: implement handle_duplex
//
// Target signature:
//
// ```rust,ignore
// pub async fn handle_duplex(
//     conductor: sacp_conductor::ConductorImpl<sacp::Agent>,
//     stream: tokio::io::DuplexStream,
// ) -> anyhow::Result<()>;
// ```
