mod auth;
mod heartbeat;
mod local_provider;
mod router;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use fireline_runtime::{
    DockerProvider, DockerProviderConfig, LocalProvider, RuntimeHost, RuntimeManager,
    RuntimeRegistry, RuntimeTokenIssuer,
};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::EnvFilter;

use self::auth::RuntimeTokenStore;
use self::heartbeat::HeartbeatTracker;
use self::local_provider::ChildProcessRuntimeLauncher;
use self::router::{build_router, AppState};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ProviderMode {
    Local,
    Docker,
}

#[derive(Debug, Parser)]
#[command(
    name = "fireline-control-plane",
    about = "Fireline control plane for runtime lifecycle"
)]
struct Cli {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    #[arg(long, default_value_t = 4440)]
    port: u16,

    /// Optional path to write the actual bound `host:port` to after the
    /// listener is up. Used by tests that pass `--port 0` and need to learn
    /// the OS-assigned port without a TOCTOU reservation race.
    #[arg(long)]
    listen_addr_file: Option<PathBuf>,

    #[arg(long)]
    fireline_bin: PathBuf,

    #[arg(long)]
    runtime_registry_path: Option<PathBuf>,

    #[arg(long)]
    peer_directory_path: Option<PathBuf>,

    #[arg(long, default_value_t = 20_000)]
    startup_timeout_ms: u64,

    #[arg(long, default_value_t = 10_000)]
    stop_timeout_ms: u64,

    #[arg(long, value_enum, default_value_t = ProviderMode::Local)]
    provider: ProviderMode,

    #[arg(
        long,
        env = "FIRELINE_CONTROL_PLANE_PREFER_PUSH",
        default_value_t = false
    )]
    prefer_push: bool,

    #[arg(long, default_value_t = 5_000)]
    heartbeat_scan_interval_ms: u64,

    #[arg(long, default_value_t = 30_000)]
    stale_timeout_ms: u64,

    #[arg(long)]
    shared_stream_base_url: Option<String>,

    #[arg(long)]
    docker_build_context: Option<PathBuf>,

    #[arg(long, default_value = "docker/fireline-runtime.Dockerfile")]
    dockerfile: PathBuf,

    #[arg(long, default_value = "fireline-runtime:dev")]
    docker_image: String,
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
    let host: std::net::IpAddr = cli.host.parse().context("parse control-plane host")?;

    // Bind the listener first so we know the actual port before constructing
    // anything that needs the base URL. This closes the TOCTOU race the test
    // harness used to hit when it would reserve-and-drop a port and then pass
    // the integer to this subprocess, which might lose the race to another
    // test binary starting in parallel.
    let bind_addr = SocketAddr::new(host, cli.port);
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("bind control plane listener on {bind_addr}"))?;
    let bound_addr = listener
        .local_addr()
        .context("resolve control plane bound address")?;
    let bound_port = bound_addr.port();
    let base_url = control_plane_base_url(host, bound_port);

    if let Some(listen_addr_file) = cli.listen_addr_file.as_ref() {
        if let Some(parent) = listen_addr_file.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create listen-addr-file parent {}", parent.display()))?;
        }
        std::fs::write(listen_addr_file, bound_addr.to_string()).with_context(|| {
            format!(
                "write control plane listen addr to {}",
                listen_addr_file.display()
            )
        })?;
    }

    let runtime_registry_path = match cli.runtime_registry_path {
        Some(path) => path,
        None => RuntimeRegistry::default_path()?,
    };
    let runtime_registry = RuntimeRegistry::load(runtime_registry_path.clone())?;
    let token_store = RuntimeTokenStore::default();
    let heartbeat_tracker = HeartbeatTracker::new(runtime_registry.clone());
    let launcher = Arc::new(ChildProcessRuntimeLauncher::new(
        cli.fireline_bin,
        runtime_registry.clone(),
        runtime_registry_path,
        cli.peer_directory_path,
        cli.prefer_push,
        base_url.clone(),
        cli.shared_stream_base_url.clone(),
        token_store.clone(),
        Duration::from_millis(cli.startup_timeout_ms),
        Duration::from_millis(cli.stop_timeout_ms),
    ));
    let mut runtime_manager = RuntimeManager::new(Arc::new(LocalProvider::new(launcher)));
    if matches!(cli.provider, ProviderMode::Docker) {
        let build_context = cli
            .docker_build_context
            .clone()
            .unwrap_or(default_repo_root()?);
        let docker_provider = Arc::new(DockerProvider::new(
            DockerProviderConfig {
                control_plane_url: base_url.clone(),
                shared_stream_base_url: cli.shared_stream_base_url.clone(),
                image: cli.docker_image.clone(),
                build_context,
                dockerfile: cli.dockerfile.clone(),
            },
            Arc::new(ControlPlaneTokenIssuer {
                token_store: token_store.clone(),
            }),
        )?);
        runtime_manager = runtime_manager.with_provider(docker_provider);
    }
    let runtime_host = RuntimeHost::new(runtime_registry.clone(), runtime_manager);
    spawn_stale_runtime_task(
        runtime_registry.clone(),
        heartbeat_tracker.clone(),
        Duration::from_millis(cli.heartbeat_scan_interval_ms),
        Duration::from_millis(cli.stale_timeout_ms),
    );

    let app = build_router(AppState {
        runtime_host,
        heartbeat_tracker,
        token_store,
    });

    tracing::info!(addr = %bound_addr, "fireline control plane listening");
    axum::serve(listener, app)
        .await
        .context("serve control plane")
}

fn spawn_stale_runtime_task(
    runtime_registry: RuntimeRegistry,
    heartbeat_tracker: HeartbeatTracker,
    scan_interval: Duration,
    stale_timeout: Duration,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(scan_interval);
        loop {
            interval.tick().await;
            let stale_before_ms = now_ms() - stale_timeout.as_millis() as i64;
            let stale_keys = match heartbeat_tracker.stale_keys(stale_before_ms).await {
                Ok(stale_keys) => stale_keys,
                Err(error) => {
                    tracing::warn!(?error, "scan stale runtime heartbeats");
                    continue;
                }
            };
            for runtime_key in stale_keys {
                let runtime = match runtime_registry.get(&runtime_key) {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        tracing::warn!(
                            ?error,
                            runtime_key,
                            "read runtime while scanning heartbeats"
                        );
                        continue;
                    }
                };

                let Some(mut descriptor) = runtime else {
                    if let Err(error) = heartbeat_tracker.forget(&runtime_key).await {
                        tracing::warn!(?error, runtime_key, "forget missing runtime heartbeat");
                    }
                    continue;
                };

                if descriptor.status != fireline_runtime::RuntimeStatus::Ready {
                    if matches!(
                        descriptor.status,
                        fireline_runtime::RuntimeStatus::Stopped
                            | fireline_runtime::RuntimeStatus::Broken
                    ) {
                        if let Err(error) = heartbeat_tracker.forget(&runtime_key).await {
                            tracing::warn!(
                                ?error,
                                runtime_key,
                                "forget finalized runtime heartbeat"
                            );
                        }
                    }
                    continue;
                }

                descriptor.status = fireline_runtime::RuntimeStatus::Stale;
                descriptor.updated_at_ms = now_ms();
                if let Err(error) = runtime_registry.upsert(descriptor) {
                    tracing::warn!(?error, runtime_key, "mark runtime stale");
                }
            }
        }
    });
}

fn control_plane_base_url(host: std::net::IpAddr, port: u16) -> String {
    let connect_host = if host.is_unspecified() {
        match host {
            std::net::IpAddr::V4(_) => "127.0.0.1".to_string(),
            std::net::IpAddr::V6(_) => "::1".to_string(),
        }
    } else {
        host.to_string()
    };
    format!("http://{connect_host}:{port}")
}

fn default_repo_root() -> Result<PathBuf> {
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("resolve control-plane crate parent")?
        .parent()
        .context("resolve workspace root")?
        .to_path_buf())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}

#[derive(Clone)]
struct ControlPlaneTokenIssuer {
    token_store: RuntimeTokenStore,
}

impl RuntimeTokenIssuer for ControlPlaneTokenIssuer {
    fn issue(&self, runtime_key: &str, ttl: Duration) -> String {
        self.token_store.issue(runtime_key, ttl).token
    }
}
