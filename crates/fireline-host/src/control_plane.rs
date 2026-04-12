use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use fireline_sandbox::{
    DockerProvider, DockerProviderConfig, LocalProvider, RuntimeRegistry, SandboxDispatcher,
    SandboxTokenIssuer,
};
use fireline_session::HostIndex;

use crate::auth::RuntimeTokenStore;
use crate::local_provider::ChildProcessSandboxLauncher;
use crate::router::{AppState, HostInfraConfig, build_router};

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
    pub durable_streams_url: String,
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
    let peer_directory_path = config.peer_directory_path.clone();
    let launcher = Arc::new(ChildProcessSandboxLauncher::new(
        config.fireline_bin,
        runtime_registry.clone(),
        runtime_registry_path,
        config.peer_directory_path,
        config.prefer_push,
        base_url.clone(),
        token_store.clone(),
        config.startup_timeout,
        config.stop_timeout,
    ));
    let read_model = Arc::new(HostIndex::new());
    let mut dispatcher =
        SandboxDispatcher::new(read_model, Arc::new(LocalProvider::new(launcher)));
    if matches!(config.provider, ProviderMode::Docker) {
        let build_context = config
            .docker_build_context
            .clone()
            .unwrap_or(default_repo_root()?);
        let docker_provider = Arc::new(DockerProvider::new(
            DockerProviderConfig {
                control_plane_url: base_url.clone(),
                image: config.docker_image.clone(),
                build_context,
                dockerfile: config.dockerfile.clone(),
            },
            Arc::new(ControlPlaneTokenIssuer {
                token_store: token_store.clone(),
            }),
        )?);
        dispatcher = dispatcher.with_provider(docker_provider);
    }

    let app = build_router(AppState {
        dispatcher,
        infra: HostInfraConfig {
            host: config.host,
            durable_streams_url: config.durable_streams_url.clone(),
            peer_directory_path,
        },
    });

    tracing::info!(addr = %bound_addr, "fireline control plane listening");
    axum::serve(listener, app)
        .await
        .context("serve control plane")
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

#[derive(Clone)]
struct ControlPlaneTokenIssuer {
    token_store: RuntimeTokenStore,
}

impl SandboxTokenIssuer for ControlPlaneTokenIssuer {
    fn issue(&self, host_key: &str, ttl: Duration) -> String {
        self.token_store.issue(host_key, ttl).token
    }
}
