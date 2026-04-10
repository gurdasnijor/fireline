mod auth;
mod heartbeat;
mod local_provider;
mod router;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use fireline_conductor::runtime::{LocalProvider, RuntimeHost, RuntimeManager, RuntimeRegistry};
use tracing_subscriber::EnvFilter;

use self::auth::RuntimeTokenStore;
use self::heartbeat::HeartbeatTracker;
use self::local_provider::ChildProcessRuntimeLauncher;
use self::router::{AppState, build_router};

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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .without_time()
        .init();

    let cli = Cli::parse();
    let host: std::net::IpAddr = cli.host.parse().context("parse control-plane host")?;
    let runtime_registry_path = match cli.runtime_registry_path {
        Some(path) => path,
        None => RuntimeRegistry::default_path()?,
    };
    let runtime_registry = RuntimeRegistry::load(runtime_registry_path.clone())?;
    let token_store = RuntimeTokenStore::default();
    let heartbeat_tracker = HeartbeatTracker::new();
    let launcher = Arc::new(ChildProcessRuntimeLauncher::new(
        cli.fireline_bin,
        runtime_registry.clone(),
        runtime_registry_path,
        cli.peer_directory_path,
        cli.prefer_push,
        control_plane_base_url(host, cli.port),
        token_store.clone(),
        Duration::from_millis(cli.startup_timeout_ms),
        Duration::from_millis(cli.stop_timeout_ms),
    ));
    let runtime_host = RuntimeHost::new(
        runtime_registry.clone(),
        RuntimeManager::new(Arc::new(LocalProvider::new(launcher))),
    );
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

    let addr = SocketAddr::new(host, cli.port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind control plane listener on {addr}"))?;

    tracing::info!(addr = %addr, "fireline control plane listening");
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
            for runtime_key in heartbeat_tracker.stale_keys(stale_before_ms).await {
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
                    heartbeat_tracker.forget(&runtime_key).await;
                    continue;
                };

                if descriptor.status != fireline_conductor::runtime::RuntimeStatus::Ready {
                    if matches!(
                        descriptor.status,
                        fireline_conductor::runtime::RuntimeStatus::Stopped
                            | fireline_conductor::runtime::RuntimeStatus::Broken
                    ) {
                        heartbeat_tracker.forget(&runtime_key).await;
                    }
                    continue;
                }

                descriptor.status = fireline_conductor::runtime::RuntimeStatus::Stale;
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

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}
