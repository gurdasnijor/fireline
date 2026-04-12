//! Standalone durable-streams server for local development.
//!
//! Embeds the same `build_stream_router()` the test harness uses,
//! bound to a fixed port so dev-server.mjs can spawn it and point
//! child runtimes at it. Production deployments use the external
//! durable-streams-server Docker image instead.

use anyhow::{Context, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7474);

    let router = fireline_session::build_stream_router(None)?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .context("bind durable-streams listener")?;

    let addr = listener.local_addr()?;
    eprintln!("durable-streams ready at http://127.0.0.1:{}/v1/stream", addr.port());

    axum::serve(listener, router)
        .await
        .context("durable-streams server exited")
}
