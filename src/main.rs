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
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "fireline", about = "Fireline runtime substrate for ACP-compatible agents")]
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

    let _cli = Cli::parse();

    // TODO: call fireline::bootstrap::start(config) and wait for shutdown.
    //
    // ```rust,ignore
    // let handle = fireline::bootstrap::start(BootstrapConfig {
    //     host: cli.host.parse()?,
    //     port: cli.port,
    //     name: cli.name,
    //     agent_command: cli.agent_command,
    // }).await?;
    //
    // tokio::signal::ctrl_c().await.ok();
    // handle.shutdown().await?;
    // ```

    Ok(())
}
