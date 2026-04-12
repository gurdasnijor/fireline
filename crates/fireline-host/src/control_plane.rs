use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use fireline_sandbox::{
    DockerProvider, DockerProviderConfig, LocalProvider, RuntimeHost, RuntimeManager,
    RuntimeRegistry, RuntimeStatus, RuntimeTokenIssuer,
};

use crate::auth::RuntimeTokenStore;
use crate::heartbeat::HeartbeatTracker;
use crate::local_provider::ChildProcessRuntimeLauncher;
use crate::router::{AppState, build_router};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderMode {
    Local,
    Docker,
}

#[derive(Clone, Debug)]
pub struct ControlPlaneConfig {
    pub host: std::net::IpAddr,
    pub port: u16,
    pub listen_addr_file: Option<PathBuf>,
    pub fireline_bin: PathBuf,
    pub runtime_registry_path: Option<PathBuf>,
    pub peer_directory_path: Option<PathBuf>,
    pub startup_timeout: Duration,
    pub stop_timeout: Duration,
    pub provider: ProviderMode,
    pub prefer_push: bool,
    pub heartbeat_scan_interval: Duration,
    pub stale_timeout: Duration,
    pub shared_stream_base_url: Option<String>,
    pub docker_build_context: Option<PathBuf>,
    pub dockerfile: PathBuf,
    pub docker_image: String,
}

pub async fn run_control_plane(config: ControlPlaneConfig) -> Result<()> {
    let bind_addr = SocketAddr::new(config.host, config.port);
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("bind control plane listener on {bind_addr}"))?;
    let bound_addr = listener
        .local_addr()
        .context("resolve control plane bound address")?;
    let bound_port = bound_addr.port();
    let base_url = control_plane_base_url(config.host, bound_port);

    if let Some(listen_addr_file) = config.listen_addr_file.as_ref() {
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

    let runtime_registry_path = match config.runtime_registry_path {
        Some(path) => path,
        None => RuntimeRegistry::default_path()?,
    };
    let runtime_registry = RuntimeRegistry::load(runtime_registry_path.clone())?;
    let token_store = RuntimeTokenStore::default();
    let heartbeat_tracker = HeartbeatTracker::new(runtime_registry.clone());
    let launcher = Arc::new(ChildProcessRuntimeLauncher::new(
        config.fireline_bin,
        runtime_registry.clone(),
        runtime_registry_path,
        config.peer_directory_path,
        config.prefer_push,
        base_url.clone(),
        config.shared_stream_base_url.clone(),
        token_store.clone(),
        config.startup_timeout,
        config.stop_timeout,
    ));
    let mut runtime_manager = RuntimeManager::new(Arc::new(LocalProvider::new(launcher)));
    if matches!(config.provider, ProviderMode::Docker) {
        let build_context = config
            .docker_build_context
            .clone()
            .unwrap_or(default_repo_root()?);
        let docker_provider = Arc::new(DockerProvider::new(
            DockerProviderConfig {
                control_plane_url: base_url.clone(),
                shared_stream_base_url: config.shared_stream_base_url.clone(),
                image: config.docker_image.clone(),
                build_context,
                dockerfile: config.dockerfile.clone(),
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
        config.heartbeat_scan_interval,
        config.stale_timeout,
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

                if descriptor.status != RuntimeStatus::Ready {
                    if matches!(
                        descriptor.status,
                        RuntimeStatus::Stopped | RuntimeStatus::Broken
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

                descriptor.status = RuntimeStatus::Stale;
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
        .context("resolve host crate parent")?
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
