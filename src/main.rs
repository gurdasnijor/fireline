//! Fireline CLI entry point.
//!
//! Parses CLI args, calls [`fireline_runtime::bootstrap::start`], waits for
//! the shutdown signal, and exits. Should stay under ~50 lines.
//!
//! All runtime assembly lives in the primitive crates, not in a root
//! `fireline` library shim.

use anyhow::{Context, Result};
use clap::Parser;
use fireline_runtime::bootstrap::{self, BootstrapConfig};
use fireline_runtime::control_plane_client::ControlPlaneClient;
use fireline_runtime::runtime_host::{Endpoint, RuntimeDescriptor, RuntimeProviderKind, RuntimeStatus};
use fireline_runtime::RuntimeRegistry;
use fireline_runtime::{HeartbeatMetrics, MountedResource, RuntimeRegistration};
use fireline_harness::TopologySpec;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::fmt::format::FmtSpan;
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
    #[arg(long, env = "FIRELINE_RUNTIME_KEY", hide = true)]
    runtime_key: Option<String>,

    /// Optional explicit node id for control-plane-managed subprocess mode.
    #[arg(long, env = "FIRELINE_NODE_ID", hide = true)]
    node_id: Option<String>,

    /// Optional control-plane base URL used by managed runtimes in push mode.
    #[arg(long, env = "FIRELINE_CONTROL_PLANE_URL", hide = true)]
    control_plane_url: Option<String>,

    /// Optional provider kind override for control-plane-managed runtimes.
    #[arg(long, env = "FIRELINE_PROVIDER_KIND", hide = true)]
    provider_kind: Option<String>,

    /// Optional provider instance id override for control-plane-managed runtimes.
    #[arg(long, env = "FIRELINE_PROVIDER_INSTANCE_ID", hide = true)]
    provider_instance_id: Option<String>,

    /// Optional externally reachable ACP URL to register instead of the bind URL.
    #[arg(long, env = "FIRELINE_ADVERTISED_ACP_URL", hide = true)]
    advertised_acp_url: Option<String>,

    /// Optional externally reachable state stream URL to register instead of the bind URL.
    #[arg(long, env = "FIRELINE_ADVERTISED_STATE_STREAM_URL", hide = true)]
    advertised_state_stream_url: Option<String>,

    /// Optional external durable-streams URL for this runtime's state stream.
    #[arg(long, env = "FIRELINE_EXTERNAL_STATE_STREAM_URL", hide = true)]
    external_state_stream_url: Option<String>,

    /// Optional runtime topology JSON payload.
    #[arg(long)]
    topology_json: Option<String>,

    /// Optional normalized resource mounts prepared by the provider.
    #[arg(long, hide = true)]
    mounted_resources_json: Option<String>,

    /// The agent command to run, e.g. `npx -y @zed-industries/claude-code-acp`.
    #[arg(trailing_var_arg = true, required = true)]
    agent_command: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let span_events = if std::env::var_os("FIRELINE_TRACE_SPANS").is_some() {
        FmtSpan::CLOSE
    } else {
        FmtSpan::NONE
    };
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_span_events(span_events)
        .without_time()
        .init();

    let cli = Cli::parse();
    let host: IpAddr = cli.host.parse()?;
    let topology = match cli.topology_json {
        Some(ref json) => serde_json::from_str::<TopologySpec>(json)?,
        None => TopologySpec::default(),
    };
    let mounted_resources = match cli.mounted_resources_json.as_deref() {
        Some(json) => serde_json::from_str::<Vec<MountedResource>>(json)?,
        None => Vec::new(),
    };
    let managed_runtime_key = cli.runtime_key.clone();
    let managed_node_id = cli.node_id.clone();

    match (managed_runtime_key, managed_node_id) {
        (Some(runtime_key), Some(node_id)) => {
            run_managed_runtime(cli, host, topology, mounted_resources, runtime_key, node_id).await
        }
        (None, None) => {
            let registry = load_runtime_registry(cli.runtime_registry_path.clone())?;
            run_direct_host(cli, host, topology, mounted_resources, registry).await
        }
        _ => Err(anyhow::anyhow!(
            "--runtime-key and --node-id must be provided together"
        )),
    }
}

async fn run_direct_host(
    cli: Cli,
    host: IpAddr,
    topology: TopologySpec,
    _mounted_resources: Vec<MountedResource>,
    registry: RuntimeRegistry,
) -> Result<()> {
    let runtime_host = fireline_runtime::runtime_host::RuntimeHost::new(registry);
    let descriptor = runtime_host
        .create(fireline_runtime::runtime_host::CreateRuntimeSpec {
            runtime_key: None,
            node_id: None,
            provider: fireline_runtime::runtime_host::RuntimeProviderRequest::Local,
            host,
            port: cli.port,
            name: cli.name,
            agent_command: cli.agent_command,
            resources: Vec::new(),
            state_stream: cli.state_stream,
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
    mounted_resources: Vec<MountedResource>,
    runtime_key: String,
    node_id: String,
) -> Result<()> {
    let peer_directory_path = match cli.peer_directory_path {
        Some(path) => path,
        None => fireline_tools::LocalPeerDirectory::default_path()?,
    };
    let started_at_ms = now_ms();
    let handle = bootstrap::start(BootstrapConfig {
        host,
        port: cli.port,
        name: cli.name,
        runtime_key: runtime_key.clone(),
        node_id: node_id.clone(),
        agent_command: cli.agent_command,
        mounted_resources,
        state_stream: cli.state_stream,
        stream_storage: None,
        peer_directory_path,
        control_plane_url: cli.control_plane_url.clone(),
        external_state_stream_url: cli.external_state_stream_url.clone(),
        topology,
    })
    .await?;
    wait_for_runtime_listener_ready(&handle.health_url).await?;

    let provider = parse_provider_kind(cli.provider_kind.as_deref())?;
    let advertised_acp_url = cli
        .advertised_acp_url
        .clone()
        .unwrap_or_else(|| handle.acp_url.clone());
    let advertised_state_stream_url = cli
        .advertised_state_stream_url
        .clone()
        .or(cli.external_state_stream_url.clone())
        .unwrap_or_else(|| handle.state_stream_url.clone());
    let descriptor = RuntimeDescriptor {
        runtime_key: runtime_key.clone(),
        runtime_id: handle.runtime_id.clone(),
        node_id,
        provider,
        provider_instance_id: cli
            .provider_instance_id
            .clone()
            .unwrap_or_else(|| handle.runtime_id.clone()),
        status: RuntimeStatus::Ready,
        acp: Endpoint::new(advertised_acp_url),
        state: Endpoint::new(advertised_state_stream_url),
        helper_api_base_url: None,
        created_at_ms: started_at_ms,
        updated_at_ms: started_at_ms,
    };

    let heartbeat_task = if let Some(control_plane_url) = cli.control_plane_url.clone() {
        let token = std::env::var("FIRELINE_CONTROL_PLANE_TOKEN")
            .context("FIRELINE_CONTROL_PLANE_TOKEN is required in push mode")?;
        let control_plane_client = Arc::new(ControlPlaneClient::new(
            control_plane_url,
            token,
            runtime_key,
        )?);
        control_plane_client
            .register(RuntimeRegistration {
                runtime_id: descriptor.runtime_id.clone(),
                node_id: descriptor.node_id.clone(),
                provider: descriptor.provider,
                provider_instance_id: descriptor.provider_instance_id.clone(),
                advertised_acp_url: descriptor.acp.url.clone(),
                advertised_state_stream_url: descriptor.state.url.clone(),
                helper_api_base_url: descriptor.helper_api_base_url.clone(),
            })
            .await?;
        Some(control_plane_client.spawn_heartbeat_loop(HeartbeatMetrics::default))
    } else {
        let registry = load_runtime_registry(cli.runtime_registry_path.clone())?;
        registry.upsert(descriptor.clone())?;
        None
    };

    log_runtime_started(&descriptor);
    tokio::signal::ctrl_c().await.ok();
    if let Some(task) = heartbeat_task {
        task.abort();
        let _ = task.await;
    }
    handle.shutdown().await?;

    if cli.control_plane_url.is_none() {
        let registry = load_runtime_registry(cli.runtime_registry_path.clone())?;
        let mut stopped = descriptor;
        stopped.status = RuntimeStatus::Stopped;
        stopped.updated_at_ms = now_ms();
        registry.upsert(stopped)?;
    }
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
        acp_url = %descriptor.acp.url,
        state_stream_url = %descriptor.state.url,
        "fireline runtime started"
    );
}

async fn wait_for_runtime_listener_ready(health_url: &str) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .context("build runtime healthcheck client")?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match client.get(health_url).send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(_) | Err(_) if tokio::time::Instant::now() >= deadline => {
                return Err(anyhow::anyhow!(
                    "runtime listener did not become healthy before registration"
                ));
            }
            Ok(_) | Err(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}

fn parse_provider_kind(value: Option<&str>) -> Result<RuntimeProviderKind> {
    match value {
        None | Some("local") => Ok(RuntimeProviderKind::Local),
        Some("docker") => Ok(RuntimeProviderKind::Docker),
        Some(other) => Err(anyhow::anyhow!(
            "unsupported runtime provider kind '{other}'"
        )),
    }
}
