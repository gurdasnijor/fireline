use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use fireline_harness::resolve_spawn_env_vars;
use fireline_resources::ResourceRef;
use fireline_sandbox::{ProviderDispatcher, SandboxConfig, SandboxDescriptor, SandboxHandle};
use fireline_session::TopologySpec;
use serde::{Deserialize, Serialize};

/// Host-side infrastructure config — never sent by clients.
#[derive(Clone, Debug)]
pub struct HostInfraConfig {
    pub durable_streams_url: String,
    pub control_plane_url: String,
}

#[derive(Clone)]
pub struct AppState {
    pub dispatcher: ProviderDispatcher,
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
    #[serde(default)]
    pub env_vars: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub labels: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub provider: Option<String>,
    pub state_stream: Option<String>,
    #[serde(default)]
    pub topology: TopologySpec,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/sandboxes", get(list_sandboxes).post(provision_sandbox))
        .route(
            "/v1/sandboxes/{sandbox_id}",
            get(get_sandbox).delete(delete_sandbox),
        )
        .route("/v1/sandboxes/{sandbox_id}/stop", post(stop_sandbox))
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn list_sandboxes(
    State(state): State<AppState>,
) -> Result<Json<Vec<SandboxDescriptor>>, ControlPlaneError> {
    Ok(Json(state.dispatcher.list(None).await?))
}

async fn get_sandbox(
    Path(sandbox_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<SandboxDescriptor>, ControlPlaneError> {
    let sandbox =
        state.dispatcher.get(&sandbox_id).await?.ok_or_else(|| {
            ControlPlaneError::not_found(format!("sandbox '{sandbox_id}' not found"))
        })?;
    Ok(Json(sandbox))
}

async fn provision_sandbox(
    State(state): State<AppState>,
    Json(request): Json<ProvisionRequest>,
) -> Result<(StatusCode, Json<SandboxHandle>), ControlPlaneError> {
    let mut env_vars = request.env_vars;
    env_vars.extend(resolve_spawn_env_vars(&request.topology)?);
    let config = SandboxConfig {
        name: request.name,
        agent_command: request.agent_command,
        topology: request.topology,
        resources: request.resources,
        durable_streams_url: state.infra.durable_streams_url.clone(),
        state_stream: request.state_stream,
        env_vars,
        control_plane_url: Some(state.infra.control_plane_url.clone()),
        labels: request.labels,
        provider: request.provider,
    };
    let sandbox = state.dispatcher.create(config).await?;
    Ok((StatusCode::CREATED, Json(sandbox)))
}

async fn stop_sandbox(
    Path(sandbox_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<SandboxDescriptor>, ControlPlaneError> {
    let sandbox =
        state.dispatcher.stop(&sandbox_id).await?.ok_or_else(|| {
            ControlPlaneError::not_found(format!("sandbox '{sandbox_id}' not found"))
        })?;
    Ok(Json(sandbox))
}

async fn delete_sandbox(
    Path(sandbox_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<SandboxDescriptor>, ControlPlaneError> {
    let sandbox =
        state.dispatcher.stop(&sandbox_id).await?.ok_or_else(|| {
            ControlPlaneError::not_found(format!("sandbox '{sandbox_id}' not found"))
        })?;
    Ok(Json(sandbox))
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
