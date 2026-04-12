use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use fireline_sandbox::{
    DockerProvider, DockerProviderConfig, LocalSubprocessProvider, LocalSubprocessProviderConfig,
    ProviderDispatcher,
};
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
}

pub async fn run_host(config: HostConfig) -> Result<()> {
    let bind_addr = SocketAddr::new(config.host, config.port);
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("bind host listener on {bind_addr}"))?;
    let bound_addr = listener.local_addr().context("resolve host bound address")?;

    let read_model = Arc::new(HostIndex::new());
    let local_provider = Arc::new(LocalSubprocessProvider::new(LocalSubprocessProviderConfig {
        fireline_bin: config.fireline_bin.clone(),
        host: config.host,
        default_peer_directory_path: config.peer_directory_path.clone(),
        startup_timeout: config.startup_timeout,
        stop_timeout: config.stop_timeout,
    }));
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

    let app = build_router(AppState {
        dispatcher,
        infra: HostInfraConfig {
            durable_streams_url: config.durable_streams_url.clone(),
        },
    });

    tracing::info!(addr = %bound_addr, "fireline host listening");
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
