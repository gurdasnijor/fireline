use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use fireline_sandbox::{
    DockerProvider, DockerProviderConfig, LocalSubprocessProvider, LocalSubprocessProviderConfig,
    ProviderDispatcher,
};
#[cfg(feature = "anthropic-provider")]
use fireline_sandbox::{RemoteAnthropicProvider, RemoteAnthropicProviderConfig};
use fireline_session::HostIndex;

use crate::router::{AppState, HostInfraConfig, build_router};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderMode {
    Local,
    Docker,
}

#[derive(Clone, Debug)]
pub struct HostConfig {
    pub host: std::net::IpAddr,
    pub port: u16,
    pub fireline_bin: PathBuf,
    pub peer_directory_path: Option<PathBuf>,
    pub startup_timeout: Duration,
    pub stop_timeout: Duration,
    pub provider: ProviderMode,
    pub durable_streams_url: String,
    pub docker_build_context: Option<PathBuf>,
    pub dockerfile: PathBuf,
    pub docker_image: String,
    /// Optional path to write the bound listener address to after binding.
    /// Enables tests that bind on port 0 to discover the actual bound port.
    pub listen_addr_file: Option<PathBuf>,
}

pub async fn run_host(config: HostConfig) -> Result<()> {
    let bind_addr = SocketAddr::new(config.host, config.port);
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("bind host listener on {bind_addr}"))?;
    let bound_addr = listener
        .local_addr()
        .context("resolve host bound address")?;
    let control_plane_url = format!(
        "http://{}:{}",
        connect_host(bound_addr.ip()),
        bound_addr.port()
    );

    let read_model = Arc::new(HostIndex::new());
    let local_provider = Arc::new(LocalSubprocessProvider::new(
        LocalSubprocessProviderConfig {
            fireline_bin: config.fireline_bin.clone(),
            host: config.host,
            default_peer_directory_path: config.peer_directory_path.clone(),
            startup_timeout: config.startup_timeout,
            stop_timeout: config.stop_timeout,
        },
    ));
    let dispatcher = match config.provider {
        ProviderMode::Local => ProviderDispatcher::new(local_provider, read_model),
        ProviderMode::Docker => {
            let build_context = config
                .docker_build_context
                .clone()
                .unwrap_or(default_repo_root()?);
            let docker_provider = Arc::new(DockerProvider::new(DockerProviderConfig {
                image: config.docker_image.clone(),
                build_context,
                dockerfile: config.dockerfile.clone(),
                startup_timeout: config.startup_timeout,
            })?);
            ProviderDispatcher::new(docker_provider, read_model).with_provider(local_provider)
        }
    };
    #[cfg(feature = "anthropic-provider")]
    let dispatcher = dispatcher.with_provider(Arc::new(RemoteAnthropicProvider::new(
        RemoteAnthropicProviderConfig::default(),
    )?));

    let app = build_router(AppState {
        dispatcher,
        infra: HostInfraConfig {
            durable_streams_url: config.durable_streams_url.clone(),
            control_plane_url,
        },
    });

    tracing::info!(addr = %bound_addr, "fireline host listening");

    if let Some(path) = &config.listen_addr_file {
        std::fs::write(path, bound_addr.to_string())
            .with_context(|| format!("write listen-addr-file at {}", path.display()))?;
    }

    axum::serve(listener, app).await.context("serve host")
}

fn default_repo_root() -> Result<PathBuf> {
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("resolve host crate parent")?
        .parent()
        .context("resolve workspace root")?
        .to_path_buf())
}

fn connect_host(ip: std::net::IpAddr) -> String {
    if ip.is_unspecified() {
        match ip {
            std::net::IpAddr::V4(_) => "127.0.0.1".to_string(),
            std::net::IpAddr::V6(_) => "::1".to_string(),
        }
    } else {
        ip.to_string()
    }
}
