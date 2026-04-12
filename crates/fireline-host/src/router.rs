use std::net::IpAddr;
use std::path::PathBuf;

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use fireline_resources::ResourceRef;
use fireline_sandbox::RuntimeHost;
use fireline_session::{
    CreateRuntimeSpec, HeartbeatReport, RuntimeDescriptor, RuntimeRegistration,
    RuntimeProviderRequest, RuntimeStatus, TopologySpec,
};
use serde::{Deserialize, Serialize};

use crate::auth::{RuntimeTokenClaims, RuntimeTokenStore, require_runtime_bearer};
use crate::heartbeat::HeartbeatTracker;

/// Host-side infrastructure config — never sent by clients.
#[derive(Clone, Debug)]
pub struct HostInfraConfig {
    pub host: IpAddr,
    pub durable_streams_url: String,
    pub peer_directory_path: Option<PathBuf>,
}

#[derive(Clone)]
pub struct AppState {
    pub runtime_host: RuntimeHost,
    pub heartbeat_tracker: HeartbeatTracker,
    pub token_store: RuntimeTokenStore,
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
    let protected_runtime_routes = Router::new()
        .route("/{runtime_key}/register", post(register_runtime))
        .route("/{runtime_key}/heartbeat", post(heartbeat_runtime))
        .route_layer(axum::middleware::from_fn_with_state(
            state.token_store.clone(),
            require_runtime_bearer,
        ));

    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/runtimes", get(list_runtimes).post(provision_runtime))
        .route(
            "/v1/runtimes/{runtime_key}",
            get(get_runtime).delete(delete_runtime),
        )
        .route("/v1/runtimes/{runtime_key}/stop", post(stop_runtime))
        .nest("/v1/runtimes", protected_runtime_routes)
        .with_state(state)
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

async fn provision_runtime(
    State(state): State<AppState>,
    Json(request): Json<ProvisionRequest>,
) -> Result<(StatusCode, Json<RuntimeDescriptor>), ControlPlaneError> {
    let spec = CreateRuntimeSpec {
        runtime_key: None,
        node_id: None,
        provider: RuntimeProviderRequest::Local,
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
    let runtime = state.runtime_host.provision(spec).await?;
    Ok((StatusCode::CREATED, Json(runtime)))
}

async fn stop_runtime(
    Path(runtime_key): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<RuntimeDescriptor>, ControlPlaneError> {
    let runtime = state.runtime_host.stop(&runtime_key).await?;
    if let Err(error) = state.heartbeat_tracker.forget(&runtime_key).await {
        tracing::warn!(?error, runtime_key, "forget liveness after stop");
    }
    Ok(Json(runtime))
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
    if let Err(error) = state.heartbeat_tracker.forget(&runtime_key).await {
        tracing::warn!(?error, runtime_key, "forget liveness after delete");
    }
    Ok(Json(runtime))
}

async fn register_runtime(
    Path(runtime_key): Path<String>,
    Extension(claims): Extension<RuntimeTokenClaims>,
    State(state): State<AppState>,
    Json(registration): Json<RuntimeRegistration>,
) -> Result<StatusCode, ControlPlaneError> {
    enforce_runtime_scope(&claims, &runtime_key)?;
    if matches!(
        state
            .runtime_host
            .get(&runtime_key)?
            .map(|runtime| runtime.status),
        Some(RuntimeStatus::Stopped)
    ) {
        return Err(ControlPlaneError::conflict(format!(
            "runtime '{runtime_key}' is stopped and cannot re-register"
        )));
    }

    state
        .runtime_host
        .register(&runtime_key, registration)
        .await?;
    if let Err(error) = state.heartbeat_tracker.record(&runtime_key, now_ms()).await {
        tracing::warn!(?error, runtime_key, "record liveness after register");
    }
    Ok(StatusCode::OK)
}

async fn heartbeat_runtime(
    Path(runtime_key): Path<String>,
    Extension(claims): Extension<RuntimeTokenClaims>,
    State(state): State<AppState>,
    Json(report): Json<HeartbeatReport>,
) -> Result<StatusCode, ControlPlaneError> {
    enforce_runtime_scope(&claims, &runtime_key)?;
    let current = state.runtime_host.get(&runtime_key)?.ok_or_else(|| {
        ControlPlaneError::not_found(format!("runtime '{runtime_key}' not found"))
    })?;
    if matches!(
        current.status,
        RuntimeStatus::Stopped | RuntimeStatus::Broken
    ) {
        return Err(ControlPlaneError::gone(format!(
            "runtime '{runtime_key}' cannot heartbeat from status '{:?}'",
            current.status
        )));
    }

    state.runtime_host.heartbeat(&runtime_key, report)?;
    if let Err(error) = state.heartbeat_tracker.record(&runtime_key, now_ms()).await {
        tracing::warn!(?error, runtime_key, "record liveness after heartbeat");
    }
    Ok(StatusCode::OK)
}

fn enforce_runtime_scope(
    claims: &RuntimeTokenClaims,
    runtime_key: &str,
) -> Result<(), ControlPlaneError> {
    if claims.runtime_key != runtime_key {
        return Err(ControlPlaneError::forbidden(format!(
            "token for runtime '{}' cannot access runtime '{}'",
            claims.runtime_key, runtime_key
        )));
    }
    Ok(())
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

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}
