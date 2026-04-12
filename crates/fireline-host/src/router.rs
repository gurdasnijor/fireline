use std::net::IpAddr;
use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use fireline_resources::ResourceRef;
use fireline_sandbox::SandboxDispatcher;
use fireline_session::{HostDescriptor, ProvisionSpec, SandboxProviderRequest, TopologySpec};
use serde::{Deserialize, Serialize};

/// Host-side infrastructure config — never sent by clients.
#[derive(Clone, Debug)]
pub struct HostInfraConfig {
    pub host: IpAddr,
    pub durable_streams_url: String,
    pub peer_directory_path: Option<PathBuf>,
}

#[derive(Clone)]
pub struct AppState {
    pub dispatcher: SandboxDispatcher,
    pub infra: HostInfraConfig,
}

/// Client-facing provision request — semantic intent only.
/// Infrastructure details (host, port, provider, durable-streams URL)
/// are injected by the Host from its own config, never from the client.
/// Aligned with industry patterns (Daytona, E2B, microsandbox).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionRequest {
    pub name: String,
    #[serde(default)]
    pub agent_command: Vec<String>,
    #[serde(default)]
    pub resources: Vec<ResourceRef>,
    pub state_stream: Option<String>,
    #[serde(default)]
    pub topology: TopologySpec,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/runtimes", get(list_runtimes).post(provision_runtime))
        .route(
            "/v1/runtimes/{host_key}",
            get(get_runtime).delete(delete_runtime),
        )
        .route("/v1/runtimes/{host_key}/stop", post(stop_runtime))
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn list_runtimes(
    State(state): State<AppState>,
) -> Result<Json<Vec<HostDescriptor>>, ControlPlaneError> {
    Ok(Json(state.dispatcher.list().await))
}

async fn get_runtime(
    Path(host_key): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<HostDescriptor>, ControlPlaneError> {
    let runtime = state.dispatcher.get(&host_key).await.ok_or_else(|| {
        ControlPlaneError::not_found(format!("runtime '{host_key}' not found"))
    })?;
    Ok(Json(runtime))
}

async fn provision_runtime(
    State(state): State<AppState>,
    Json(request): Json<ProvisionRequest>,
) -> Result<(StatusCode, Json<HostDescriptor>), ControlPlaneError> {
    let spec = ProvisionSpec {
        host_key: None,
        node_id: None,
        provider: SandboxProviderRequest::Local,
        host: state.infra.host,
        port: 0,
        name: request.name,
        agent_command: request.agent_command,
        durable_streams_url: state.infra.durable_streams_url.clone(),
        resources: request.resources,
        state_stream: request.state_stream,
        stream_storage: None,
        peer_directory_path: state.infra.peer_directory_path.clone(),
        topology: request.topology,
    };
    let runtime = state.dispatcher.provision(spec).await?;
    Ok((StatusCode::CREATED, Json(runtime)))
}

async fn stop_runtime(
    Path(host_key): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<HostDescriptor>, ControlPlaneError> {
    let runtime = state.dispatcher.stop(&host_key).await?;
    Ok(Json(runtime))
}

async fn delete_runtime(
    Path(host_key): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<HostDescriptor>, ControlPlaneError> {
    let runtime = state.dispatcher.stop(&host_key).await?;
    Ok(Json(runtime))
}

#[derive(Debug)]
pub struct ControlPlaneError {
    status: StatusCode,
    message: String,
}

impl ControlPlaneError {
    pub fn not_found(message: String) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message,
        }
    }

    pub fn conflict(message: String) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message,
        }
    }

    pub fn forbidden(message: String) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message,
        }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
        }
    }

    pub fn gone(message: String) -> Self {
        Self {
            status: StatusCode::GONE,
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
