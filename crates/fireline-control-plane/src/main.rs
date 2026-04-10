mod local_provider;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use fireline_conductor::runtime::{
    CreateRuntimeSpec, LocalProvider, RuntimeDescriptor, RuntimeHost, RuntimeManager,
    RuntimeRegistry,
};
use serde::Serialize;
use tracing_subscriber::EnvFilter;

use self::local_provider::ChildProcessRuntimeLauncher;

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
}

#[derive(Clone)]
struct AppState {
    runtime_host: RuntimeHost,
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
    let launcher = Arc::new(ChildProcessRuntimeLauncher::new(
        cli.fireline_bin,
        runtime_registry.clone(),
        runtime_registry_path,
        cli.peer_directory_path,
        Duration::from_millis(cli.startup_timeout_ms),
        Duration::from_millis(cli.stop_timeout_ms),
    ));
    let runtime_host = RuntimeHost::new(
        runtime_registry,
        RuntimeManager::new(Arc::new(LocalProvider::new(launcher))),
    );

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/runtimes", get(list_runtimes).post(create_runtime))
        .route(
            "/v1/runtimes/{runtime_key}",
            get(get_runtime).delete(delete_runtime),
        )
        .route("/v1/runtimes/{runtime_key}/stop", post(stop_runtime))
        .with_state(AppState { runtime_host });

    let addr = SocketAddr::new(host, cli.port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind control plane listener on {addr}"))?;

    tracing::info!(addr = %addr, "fireline control plane listening");
    axum::serve(listener, app)
        .await
        .context("serve control plane")
}

async fn healthz() -> &'static str {
    "ok"
}

async fn list_runtimes(
    State(state): State<AppState>,
) -> Result<Json<Vec<RuntimeDescriptor>>, ControlPlaneError> {
    Ok(Json(state.runtime_host.list()?))
}

async fn get_runtime(
    Path(runtime_key): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<RuntimeDescriptor>, ControlPlaneError> {
    let runtime = state.runtime_host.get(&runtime_key)?.ok_or_else(|| {
        ControlPlaneError::not_found(format!("runtime '{runtime_key}' not found"))
    })?;
    Ok(Json(runtime))
}

async fn create_runtime(
    State(state): State<AppState>,
    Json(spec): Json<CreateRuntimeSpec>,
) -> Result<(StatusCode, Json<RuntimeDescriptor>), ControlPlaneError> {
    let runtime = state.runtime_host.create(spec).await?;
    Ok((StatusCode::CREATED, Json(runtime)))
}

async fn stop_runtime(
    Path(runtime_key): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<RuntimeDescriptor>, ControlPlaneError> {
    Ok(Json(state.runtime_host.stop(&runtime_key).await?))
}

async fn delete_runtime(
    Path(runtime_key): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<RuntimeDescriptor>, ControlPlaneError> {
    let runtime = state
        .runtime_host
        .delete(&runtime_key)
        .await?
        .ok_or_else(|| {
            ControlPlaneError::not_found(format!("runtime '{runtime_key}' not found"))
        })?;
    Ok(Json(runtime))
}

struct ControlPlaneError {
    status: StatusCode,
    message: String,
}

impl ControlPlaneError {
    fn not_found(message: String) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message,
        }
    }
}

impl From<anyhow::Error> for ControlPlaneError {
    fn from(error: anyhow::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl axum::response::IntoResponse for ControlPlaneError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(ErrorResponse {
                error: self.message,
            }),
        )
            .into_response()
    }
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}
