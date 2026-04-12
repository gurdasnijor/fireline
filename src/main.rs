//! Fireline CLI entry point.
//!
//! Parses CLI args, calls [`fireline_host::bootstrap::start`], waits for
//! the shutdown signal, and exits. Should stay under ~50 lines.
//!
//! All runtime assembly lives in the primitive crates, not in a root
//! `fireline` library shim.

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use durable_streams::{Client as DurableStreamsClient, CreateOptions};
use fireline_harness::TopologySpec;
use fireline_host::bootstrap::{self, BootstrapConfig};
use fireline_host::control_plane::{self, ControlPlaneConfig, ProviderMode};
use fireline_host::control_plane_client::ControlPlaneClient;
use fireline_resources::{
    MountedResource, ResourceMetadata, ResourcePublisher, ResourceSourceRef,
    StreamResourcePublisher,
};
use fireline_sandbox::RuntimeRegistry;
use fireline_session::{
    Endpoint, HeartbeatMetrics, RuntimeDescriptor, RuntimeProviderKind, RuntimeRegistration,
    RuntimeStatus,
};
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use uuid::Uuid;

const DEFAULT_RESOURCE_TENANT_ID: &str = "default";

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ControlPlaneProvider {
    Local,
    Docker,
}

#[derive(Clone, Debug, Subcommand)]
enum FirelineCommand {
    PublishResource(PublishResourceArgs),
}

#[derive(Clone, Debug, Args)]
struct PublishResourceArgs {
    /// Local file or directory path to publish.
    path: PathBuf,

    /// Stable resource id to publish under.
    #[arg(long)]
    id: String,

    /// Base URL for the external durable-streams service, e.g. `http://127.0.0.1:8787/v1/stream`.
    #[arg(long)]
    durable_streams_url: String,

    /// Optional tag to attach to the published resource metadata. Repeat for multiple tags.
    #[arg(long = "tag")]
    tags: Vec<String>,
}

#[derive(Debug, Parser)]
#[command(
    name = "fireline",
    about = "Fireline runtime substrate for ACP-compatible agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<FirelineCommand>,

    /// Bind port for the Fireline host listener.
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

    /// Run the host/runtime HTTP API instead of bootstrapping a single ACP runtime.
    #[arg(long)]
    control_plane: bool,

    /// Base URL for the external durable-streams service, e.g. `http://127.0.0.1:8787/v1/stream`.
    #[arg(long)]
    durable_streams_url: Option<String>,

    /// Optional path to write the bound listener address after binding.
    #[arg(long)]
    listen_addr_file: Option<PathBuf>,

    /// Optional explicit path for the runtime registry file.
    #[arg(long)]
    runtime_registry_path: Option<PathBuf>,

    /// Optional explicit path for the peer directory file.
    #[arg(long)]
    peer_directory_path: Option<PathBuf>,

    /// Optional explicit path to the Fireline binary used for child runtime launches.
    #[arg(long, hide = true)]
    fireline_bin: Option<PathBuf>,

    /// Child runtime startup timeout in milliseconds.
    #[arg(long, default_value_t = 20_000)]
    startup_timeout_ms: u64,

    /// Child runtime shutdown timeout in milliseconds.
    #[arg(long, default_value_t = 10_000)]
    stop_timeout_ms: u64,

    /// Runtime provider to enable for control-plane mode.
    #[arg(long = "provider", value_enum, default_value_t = ControlPlaneProvider::Local)]
    control_plane_provider: ControlPlaneProvider,

    /// Prefer push registration over polling when managing child runtimes.
    #[arg(long)]
    prefer_push: bool,

    /// Heartbeat stale scan interval in milliseconds.
    #[arg(long, default_value_t = 5_000)]
    heartbeat_scan_interval_ms: u64,

    /// Runtime heartbeat stale timeout in milliseconds.
    #[arg(long, default_value_t = 30_000)]
    stale_timeout_ms: u64,

    /// Compatibility flag accepted for the old control-plane launcher path.
    #[arg(long, hide = true)]
    shared_stream_base_url: Option<String>,

    /// Optional Docker build context for control-plane docker mode.
    #[arg(long)]
    docker_build_context: Option<PathBuf>,

    /// Dockerfile path for control-plane docker mode.
    #[arg(long, default_value = "docker/fireline-runtime.Dockerfile")]
    dockerfile: PathBuf,

    /// Docker image tag for control-plane docker mode.
    #[arg(long, default_value = "fireline-runtime:dev")]
    docker_image: String,

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

    /// Optional runtime topology JSON payload.
    #[arg(long)]
    topology_json: Option<String>,

    /// Optional normalized resource mounts prepared by the provider.
    #[arg(long, hide = true)]
    mounted_resources_json: Option<String>,

    /// The agent command to run, e.g. `npx -y @zed-industries/claude-code-acp`.
    #[arg(trailing_var_arg = true)]
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
    if let Some(command) = cli.command.clone() {
        return match command {
            FirelineCommand::PublishResource(args) => run_publish_resource(args).await,
        };
    }

    let host: IpAddr = cli.host.parse()?;
    let topology = match cli.topology_json {
        Some(ref json) => serde_json::from_str::<TopologySpec>(json)?,
        None => TopologySpec::default(),
    };
    let mounted_resources = match cli.mounted_resources_json.as_deref() {
        Some(json) => serde_json::from_str::<Vec<MountedResource>>(json)?,
        None => Vec::new(),
    };
    if cli.control_plane {
        if cli.runtime_key.is_some() || cli.node_id.is_some() {
            anyhow::bail!("--control-plane cannot be combined with --runtime-key/--node-id");
        }
        if !cli.agent_command.is_empty() {
            anyhow::bail!("--control-plane does not accept a trailing agent command");
        }
        return run_control_plane_host(cli, host).await;
    }

    if cli.agent_command.is_empty() {
        anyhow::bail!(
            "an agent command is required unless --control-plane is set; pass it after `--`"
        );
    }

    let durable_streams_url = cli
        .durable_streams_url
        .clone()
        .context("--durable-streams-url is required unless --control-plane is set")?;
    let managed_runtime_key = cli.runtime_key.clone();
    let managed_node_id = cli.node_id.clone();

    match (managed_runtime_key, managed_node_id) {
        (Some(runtime_key), Some(node_id)) => {
            run_managed_runtime(
                cli,
                host,
                topology,
                mounted_resources,
                durable_streams_url,
                runtime_key,
                node_id,
            )
            .await
        }
        (None, None) => {
            run_direct_host(cli, host, topology, mounted_resources, durable_streams_url).await
        }
        _ => Err(anyhow::anyhow!(
            "--runtime-key and --node-id must be provided together"
        )),
    }
}

async fn run_control_plane_host(cli: Cli, host: IpAddr) -> Result<()> {
    let fireline_bin = match cli.fireline_bin {
        Some(path) => path,
        None => std::env::current_exe().context("resolve current fireline binary path")?,
    };
    let provider = match cli.control_plane_provider {
        ControlPlaneProvider::Local => ProviderMode::Local,
        ControlPlaneProvider::Docker => ProviderMode::Docker,
    };
    let durable_streams_url = cli
        .durable_streams_url
        .context("--durable-streams-url is required in control-plane mode")?;

    control_plane::run_control_plane(ControlPlaneConfig {
        host,
        port: cli.port,
        listen_addr_file: cli.listen_addr_file,
        fireline_bin,
        runtime_registry_path: cli.runtime_registry_path,
        peer_directory_path: cli.peer_directory_path,
        startup_timeout: Duration::from_millis(cli.startup_timeout_ms),
        stop_timeout: Duration::from_millis(cli.stop_timeout_ms),
        provider,
        prefer_push: cli.prefer_push,
        heartbeat_scan_interval: Duration::from_millis(cli.heartbeat_scan_interval_ms),
        stale_timeout: Duration::from_millis(cli.stale_timeout_ms),
        durable_streams_url,
        docker_build_context: cli.docker_build_context,
        dockerfile: cli.dockerfile,
        docker_image: cli.docker_image,
    })
    .await
}

struct PreparedResourceUpload {
    bytes: Vec<u8>,
    content_type: String,
    content_hash: String,
}

async fn run_publish_resource(args: PublishResourceArgs) -> Result<()> {
    validate_resource_id(&args.id)?;

    let path = args
        .path
        .canonicalize()
        .with_context(|| format!("resolve resource path {}", args.path.display()))?;
    let prepared = prepare_resource_upload(&path)?;
    let blob_key = format!("blob-{}", Uuid::new_v4());
    let blob_stream_name = format!(
        "resource-blob:tenant-{}:{}:{}",
        DEFAULT_RESOURCE_TENANT_ID,
        sanitize_stream_component(&args.id),
        blob_key
    );

    upload_blob_stream(
        &args.durable_streams_url,
        &blob_stream_name,
        &prepared.bytes,
        &prepared.content_type,
    )
    .await?;

    let publisher = StreamResourcePublisher::new(
        &args.durable_streams_url,
        DEFAULT_RESOURCE_TENANT_ID,
        default_resource_publisher_id(),
    );
    let source_ref = ResourceSourceRef::DurableStreamBlob {
        stream: blob_stream_name.clone(),
        key: blob_key.clone(),
    };
    let metadata = ResourceMetadata {
        size_bytes: Some(prepared.bytes.len() as u64),
        mime_type: Some(prepared.content_type.clone()),
        content_hash: Some(prepared.content_hash.clone()),
        tags: args.tags.clone(),
        ..ResourceMetadata::default()
    };
    publisher
        .publish_resource(args.id.clone(), source_ref, metadata)
        .await?;

    println!(
        "published resource '{}' to '{}' as DurableStreamBlob(stream='{}', key='{}')",
        args.id,
        publisher.stream_url(),
        blob_stream_name,
        blob_key
    );
    Ok(())
}

async fn run_direct_host(
    cli: Cli,
    host: IpAddr,
    topology: TopologySpec,
    mounted_resources: Vec<MountedResource>,
    durable_streams_url: String,
) -> Result<()> {
    let runtime_key = format!("runtime:{}", Uuid::new_v4());
    let node_id = default_node_id(host);
    let started_at_ms = now_ms();
    let peer_directory_path = cli.peer_directory_path.unwrap_or_default();
    let handle = bootstrap::start(BootstrapConfig {
        host,
        port: cli.port,
        name: cli.name,
        runtime_key: runtime_key.clone(),
        node_id: node_id.clone(),
        agent_command: cli.agent_command,
        mounted_resources,
        state_stream: cli.state_stream,
        durable_streams_url,
        peer_directory_path,
        control_plane_url: None,
        topology,
    })
    .await?;
    wait_for_runtime_listener_ready(&handle.health_url).await?;

    let descriptor = RuntimeDescriptor {
        runtime_key,
        runtime_id: handle.runtime_id.clone(),
        node_id,
        provider: RuntimeProviderKind::Local,
        provider_instance_id: handle.runtime_id.clone(),
        status: RuntimeStatus::Ready,
        acp: Endpoint::new(handle.acp_url.clone()),
        state: Endpoint::new(handle.state_stream_url.clone()),
        helper_api_base_url: None,
        created_at_ms: started_at_ms,
        updated_at_ms: started_at_ms,
    };

    log_runtime_started(&descriptor);
    tokio::signal::ctrl_c().await.ok();
    handle.shutdown().await
}

async fn run_managed_runtime(
    cli: Cli,
    host: IpAddr,
    topology: TopologySpec,
    mounted_resources: Vec<MountedResource>,
    durable_streams_url: String,
    runtime_key: String,
    node_id: String,
) -> Result<()> {
    let peer_directory_path = cli.peer_directory_path.unwrap_or_default();
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
        durable_streams_url,
        peer_directory_path,
        control_plane_url: cli.control_plane_url.clone(),
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

fn default_node_id(host: IpAddr) -> String {
    if host.is_unspecified() {
        "node:local".to_string()
    } else {
        format!("node:{host}")
    }
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

fn prepare_resource_upload(path: &Path) -> Result<PreparedResourceUpload> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("read resource metadata {}", path.display()))?;
    let (bytes, content_type) = if metadata.is_file() {
        (
            std::fs::read(path).with_context(|| format!("read file {}", path.display()))?,
            "application/octet-stream".to_string(),
        )
    } else if metadata.is_dir() {
        (tar_directory(path)?, "application/x-tar".to_string())
    } else {
        anyhow::bail!(
            "resource path '{}' must be a regular file or directory",
            path.display()
        );
    };

    Ok(PreparedResourceUpload {
        content_hash: sha256_hex(&bytes),
        bytes,
        content_type,
    })
}

fn tar_directory(path: &Path) -> Result<Vec<u8>> {
    let root_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("resource");
    let mut builder = tar::Builder::new(Vec::new());
    builder
        .append_dir_all(root_name, path)
        .with_context(|| format!("archive directory {}", path.display()))?;
    builder.finish().context("finish tar archive")?;
    builder.into_inner().context("extract tar archive bytes")
}

async fn upload_blob_stream(
    durable_streams_url: &str,
    blob_stream_name: &str,
    bytes: &[u8],
    content_type: &str,
) -> Result<()> {
    let client = DurableStreamsClient::new();
    let stream = client.stream(&join_stream_url(durable_streams_url, blob_stream_name));
    stream
        .create_with(
            CreateOptions::new()
                .content_type(content_type)
                .initial_data(bytes.to_vec())
                .closed(true),
        )
        .await
        .with_context(|| format!("upload blob stream '{blob_stream_name}'"))?;
    Ok(())
}

fn default_resource_publisher_id() -> String {
    if let Ok(value) = std::env::var("FIRELINE_PUBLISHER_ID")
        && !value.trim().is_empty()
    {
        return value;
    }

    let host = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "local".to_string());
    format!("cli:{host}")
}

fn validate_resource_id(id: &str) -> Result<()> {
    if id.is_empty() {
        anyhow::bail!("--id must not be empty");
    }
    if id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Ok(());
    }

    anyhow::bail!("--id must be URL-safe: use only letters, digits, '-', '_' or '.'");
}

fn sanitize_stream_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => ch,
            _ => '-',
        })
        .collect()
}

fn join_stream_url(base_url: &str, stream_name: &str) -> String {
    format!("{}/{}", base_url.trim_end_matches('/'), stream_name)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}
