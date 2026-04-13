//! Standalone durable-streams server for local development.
//!
//! Embeds the same `build_stream_router()` the test harness uses,
//! bound to a fixed port so dev-server.mjs can spawn it and point
//! child runtimes at it. Production deployments use the external
//! durable-streams-server Docker image instead.

use anyhow::{Context, Result};
use clap::Parser;
use std::net::IpAddr;

#[derive(Debug, Parser)]
#[command(
    name = "fireline-streams",
    about = "Standalone durable-streams server for local Fireline development"
)]
struct Cli {
    /// Bind port for the durable-streams listener.
    #[arg(long, env = "PORT", default_value_t = 7474)]
    port: u16,

    /// Bind address.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let host: IpAddr = cli.host.parse().context("parse durable-streams host")?;

    let router = fireline_session::build_stream_router(None)?;
    let listener = tokio::net::TcpListener::bind((host, cli.port))
        .await
        .context("bind durable-streams listener")?;

    let addr = listener.local_addr()?;
    eprintln!(
        "durable-streams ready at http://{}:{}/v1/stream",
        addr.ip(),
        addr.port()
    );

    axum::serve(listener, router)
        .await
        .context("durable-streams server exited")
}
