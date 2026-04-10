//! Fireline CLI entry point.
//!
//! Parses CLI args, calls [`fireline::bootstrap::start`], waits for
//! the shutdown signal, and exits. Should stay under ~50 lines.
//!
//! All bootstrap logic — wiring the stream server, the ACP host
//! routes, the conductor builder with components, the helper API,
//! — lives in the binary's `lib.rs` module
//! tree, not here.

use anyhow::Result;
use clap::Parser;
use fireline::bootstrap::{self, BootstrapConfig};
use fireline::runtime_host::{RuntimeDescriptor, RuntimeProviderKind, RuntimeStatus};
use fireline::runtime_registry::RuntimeRegistry;
use fireline_conductor::topology::TopologySpec;
use std::net::IpAddr;
use std::path::PathBuf;
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

    /// Optional explicit path for the runtime registry file.
    #[arg(long)]
    runtime_registry_path: Option<PathBuf>,

    /// Optional explicit path for the peer directory file.
    #[arg(long)]
    peer_directory_path: Option<PathBuf>,

    /// Optional explicit runtime key for control-plane-managed subprocess mode.
    #[arg(long, hide = true)]
    runtime_key: Option<String>,

    /// Optional explicit node id for control-plane-managed subprocess mode.
    #[arg(long, hide = true)]
    node_id: Option<String>,

    /// Optional explicit durable-streams base URL, e.g. `http://127.0.0.1:9000/v1/stream`.
    #[arg(long, hide = true)]
    external_stream_base_url: Option<String>,

    /// Optional explicit advertised ACP URL distinct from the bound listener.
    #[arg(long, hide = true)]
    advertised_acp_url: Option<String>,

    /// Optional runtime topology JSON payload.
    #[arg(long)]
    topology_json: Option<String>,

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
    let topology = match cli.topology_json {
        Some(ref json) => serde_json::from_str::<TopologySpec>(json)?,
        None => TopologySpec::default(),
    };
    let registry = load_runtime_registry(cli.runtime_registry_path.clone())?;
    let managed_runtime_key = cli.runtime_key.clone();
    let managed_node_id = cli.node_id.clone();

    match (managed_runtime_key, managed_node_id) {
        (Some(runtime_key), Some(node_id)) => {
            run_managed_runtime(cli, host, topology, registry, runtime_key, node_id).await
        }
        (None, None) => run_direct_host(cli, host, topology, registry).await,
        _ => Err(anyhow::anyhow!(
            "--runtime-key and --node-id must be provided together"
        )),
    }
}

async fn run_direct_host(
    cli: Cli,
    host: IpAddr,
    topology: TopologySpec,
    registry: RuntimeRegistry,
) -> Result<()> {
    let runtime_host = fireline::runtime_host::RuntimeHost::new(registry);
    let descriptor = runtime_host
        .create(fireline::runtime_host::CreateRuntimeSpec {
            provider: fireline::runtime_host::RuntimeProviderRequest::Local,
            host,
            port: cli.port,
            name: cli.name,
            agent_command: cli.agent_command,
            state_stream: cli.state_stream,
            external_stream_base_url: cli.external_stream_base_url,
            advertised_acp_url: cli.advertised_acp_url,
            stream_storage: None,
            peer_directory_path: cli.peer_directory_path,
            topology,
        })
        .await?;

    log_runtime_started(&descriptor);
    tokio::signal::ctrl_c().await.ok();
    runtime_host.stop(&descriptor.runtime_key).await.map(|_| ())
}

async fn run_managed_runtime(
    cli: Cli,
    host: IpAddr,
    topology: TopologySpec,
    registry: RuntimeRegistry,
    runtime_key: String,
    node_id: String,
) -> Result<()> {
    let peer_directory_path = match cli.peer_directory_path {
        Some(path) => path,
        None => fireline_components::LocalPeerDirectory::default_path()?,
    };
    let started_at_ms = now_ms();
    let handle = bootstrap::start(BootstrapConfig {
        host,
        port: cli.port,
        name: cli.name,
        runtime_key: runtime_key.clone(),
        node_id: node_id.clone(),
        agent_command: cli.agent_command,
        state_stream: cli.state_stream,
        external_stream_base_url: cli.external_stream_base_url,
        advertised_acp_url: cli.advertised_acp_url,
        stream_storage: None,
        peer_directory_path,
        topology,
    })
    .await?;

    let descriptor = RuntimeDescriptor {
        runtime_key: runtime_key.clone(),
        runtime_id: handle.runtime_id.clone(),
        node_id,
        provider: RuntimeProviderKind::Local,
        provider_instance_id: handle.runtime_id.clone(),
        status: RuntimeStatus::Ready,
        acp_url: handle.acp_url.clone(),
        state_stream_url: handle.state_stream_url.clone(),
        helper_api_base_url: None,
        created_at_ms: started_at_ms,
        updated_at_ms: started_at_ms,
    };
    registry.upsert(descriptor.clone())?;

    log_runtime_started(&descriptor);
    tokio::signal::ctrl_c().await.ok();
    handle.shutdown().await?;

    let mut stopped = descriptor;
    stopped.status = RuntimeStatus::Stopped;
    stopped.updated_at_ms = now_ms();
    registry.upsert(stopped)?;
    Ok(())
}

fn load_runtime_registry(path: Option<PathBuf>) -> Result<RuntimeRegistry> {
    match path {
        Some(path) => RuntimeRegistry::load(path),
        None => RuntimeRegistry::load(RuntimeRegistry::default_path()?),
    }
}

fn log_runtime_started(descriptor: &RuntimeDescriptor) {
    tracing::info!(
        runtime_key = %descriptor.runtime_key,
        runtime_id = %descriptor.runtime_id,
        provider = ?descriptor.provider,
        acp_url = %descriptor.acp_url,
        state_stream_url = %descriptor.state_stream_url,
        "fireline runtime started"
    );
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}
