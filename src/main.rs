//! Fireline CLI entry point.
//!
//! Parses CLI args, calls [`fireline::bootstrap::start`], waits for
//! the shutdown signal, and exits. Should stay under ~50 lines.
//!
//! All bootstrap logic — wiring the stream server, the ACP host
//! routes, the conductor builder with components, the helper API,
//! the webhook subscriber — lives in the binary's `lib.rs` module
//! tree, not here.

use anyhow::Result;
use clap::Parser;
use std::net::IpAddr;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "fireline",
    about = "Fireline runtime substrate for ACP-compatible agents"
)]
struct Cli {
    /// Bind port for the embedded durable-streams server (helper API uses port + 1).
    #[arg(long, default_value_t = 4437)]
    port: u16,

    /// Bind address.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Logical name for this Fireline instance.
    #[arg(long, default_value = "default")]
    name: String,

    /// Optional explicit name for the durable state stream.
    #[arg(long)]
    state_stream: Option<String>,

    /// The agent command to run, e.g. `npx -y @zed-industries/claude-code-acp`.
    #[arg(trailing_var_arg = true, required = true)]
    agent_command: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .without_time()
        .init();

    let cli = Cli::parse();
    let host: IpAddr = cli.host.parse()?;
    let runtime_host = fireline::runtime_host::RuntimeHost::with_default_registry()?;
    let descriptor = runtime_host
        .create(fireline::runtime_host::CreateRuntimeSpec {
            provider: fireline::runtime_host::RuntimeProviderRequest::Local,
            host,
            port: cli.port,
            name: cli.name,
            agent_command: cli.agent_command,
            state_stream: cli.state_stream,
            peer_directory_path: None,
        })
        .await?;

    tracing::info!(
        runtime_key = %descriptor.runtime_key,
        runtime_id = %descriptor.runtime_id,
        provider = ?descriptor.provider,
        acp_url = %descriptor.acp_url,
        state_stream_url = %descriptor.state_stream_url,
        "fireline runtime started"
    );

    tokio::signal::ctrl_c().await.ok();
    runtime_host.stop(&descriptor.runtime_key).await.map(|_| ())
}
