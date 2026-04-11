//! In-memory duplex transport adapter.
//!
//! Wraps a [`tokio::io::DuplexStream`] in a [`sacp::ByteStreams`] and
//! runs the conductor over it. Used in tests and for any in-process
//! integration where one side of the duplex is the conductor and the
//! other side is a test client.

use anyhow::Result;
use sacp::ByteStreams;
use sacp_conductor::ConductorImpl;
use tokio::io::DuplexStream;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

pub async fn handle_duplex(
    conductor: ConductorImpl<sacp::Agent>,
    stream: DuplexStream,
) -> Result<()> {
    let (read_half, write_half) = tokio::io::split(stream);
    conductor
        .run(ByteStreams::new(
            write_half.compat_write(),
            read_half.compat(),
        ))
        .await?;
    Ok(())
}
