use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{Method, StatusCode};
use fireline_conductor::runtime::{
    CreateRuntimeSpec, Endpoint, HeartbeatReport, LocalProvider, LocalRuntimeLauncher,
    ManagedRuntime, RuntimeDescriptor, RuntimeHost, RuntimeLaunch, RuntimeManager,
    RuntimeProviderKind, RuntimeRegistration, RuntimeRegistry, RuntimeStatus,
};
use fireline_conductor::topology::TopologySpec;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tower::util::ServiceExt;
use uuid::Uuid;

#[path = "../crates/fireline-control-plane/src/auth.rs"]
mod auth;

#[path = "../crates/fireline-control-plane/src/heartbeat.rs"]
mod heartbeat;

#[path = "../crates/fireline-control-plane/src/router.rs"]
mod router;

use auth::RuntimeTokenStore;
use heartbeat::HeartbeatTracker;
use router::{AppState, build_router};

const PRELAUNCH_PROVIDER_INSTANCE_ID: &str = "launcher-provider-instance";
const PRELAUNCH_ACP_URL: &str = "ws://127.0.0.1:4444/acp-prelaunch";
const PRELAUNCH_STATE_STREAM_URL: &str = "http://127.0.0.1:4444/v1/stream/prelaunch";
const PRELAUNCH_HELPER_API_BASE_URL: &str = "http://127.0.0.1:4444/helper-prelaunch";

#[tokio::test]
async fn register_without_authorization_header_returns_401() -> Result<()> {
    let harness = TestHarness::new()?;
    let runtime = harness.create_runtime("register-no-bearer").await?;

    let response = harness
        .post_json(
            &format!("/v1/runtimes/{}/register", runtime.runtime_key),
            registration_for(
                &runtime,
                "instance:register-no-bearer",
                "ws://127.0.0.1:5000/acp",
                "http://127.0.0.1:5000/v1/stream/fireline",
                None,
            ),
            None,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn wrong_runtime_tokens_are_rejected_on_register_and_heartbeat() -> Result<()> {
    let harness = TestHarness::new()?;
    let runtime_a = harness.create_runtime("auth-runtime-a").await?;
    let runtime_b = harness.create_runtime("auth-runtime-b").await?;
    let token_for_a = harness.issue_runtime_token(&runtime_a.runtime_key).await?;

    let register_response = harness
        .post_json(
            &format!("/v1/runtimes/{}/register", runtime_b.runtime_key),
            registration_for(
                &runtime_b,
                "instance:runtime-b",
                "ws://127.0.0.1:5001/acp",
                "http://127.0.0.1:5001/v1/stream/fireline",
                None,
            ),
            Some(&token_for_a),
        )
        .await?;
    assert!(matches!(
        register_response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ));

    let heartbeat_response = harness
        .post_json(
            &format!("/v1/runtimes/{}/heartbeat", runtime_b.runtime_key),
            HeartbeatReport {
                ts_ms: 1_000,
                metrics: None,
            },
            Some(&token_for_a),
        )
        .await?;
    assert!(matches!(
        heartbeat_response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ));

    Ok(())
}

#[tokio::test]
async fn register_on_stopped_runtime_returns_409() -> Result<()> {
    let harness = TestHarness::new()?;
    let runtime = harness.create_runtime("stopped-register").await?;
    let token = harness.issue_runtime_token(&runtime.runtime_key).await?;

    let stopped = harness.runtime_host.stop(&runtime.runtime_key).await?;
    assert_eq!(stopped.status, RuntimeStatus::Stopped);

    let response = harness
        .post_json(
            &format!("/v1/runtimes/{}/register", runtime.runtime_key),
            registration_for(
                &runtime,
                "instance:stopped-register",
                "ws://127.0.0.1:5002/acp",
                "http://127.0.0.1:5002/v1/stream/fireline",
                Some("http://127.0.0.1:5002/helper"),
            ),
            Some(&token),
        )
        .await?;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    Ok(())
}

#[tokio::test]
async fn register_round_trips_provider_identity_and_advertised_endpoints() -> Result<()> {
    let harness = TestHarness::new()?;
    let runtime = harness.create_runtime("round-trip").await?;
    let token = harness.issue_runtime_token(&runtime.runtime_key).await?;

    assert_eq!(runtime.provider_instance_id, PRELAUNCH_PROVIDER_INSTANCE_ID);
    assert_eq!(runtime.acp.url, PRELAUNCH_ACP_URL);
    assert_eq!(runtime.state.url, PRELAUNCH_STATE_STREAM_URL);
    assert_eq!(
        runtime.helper_api_base_url.as_deref(),
        Some(PRELAUNCH_HELPER_API_BASE_URL)
    );

    let provider_instance_id = "provider-instance:registered";
    let advertised_acp_url = "wss://runtime.example.test/acp";
    let advertised_state_stream_url = "https://runtime.example.test/v1/stream/fireline-state";
    let helper_api_base_url = "https://runtime.example.test/helper";

    let register_response = harness
        .post_json(
            &format!("/v1/runtimes/{}/register", runtime.runtime_key),
            registration_for(
                &runtime,
                provider_instance_id,
                advertised_acp_url,
                advertised_state_stream_url,
                Some(helper_api_base_url),
            ),
            Some(&token),
        )
        .await?;
    assert_eq!(register_response.status(), StatusCode::OK);

    let descriptor = harness.get_runtime(&runtime.runtime_key).await?;
    assert_eq!(descriptor.status, RuntimeStatus::Ready);
    assert_eq!(descriptor.runtime_id, runtime.runtime_id);
    assert_eq!(descriptor.provider_instance_id, provider_instance_id);
    assert_eq!(descriptor.acp.url, advertised_acp_url);
    assert_eq!(descriptor.state.url, advertised_state_stream_url);
    assert_eq!(
        descriptor.helper_api_base_url.as_deref(),
        Some(helper_api_base_url)
    );

    Ok(())
}

struct TestHarness {
    app: axum::Router,
    runtime_host: RuntimeHost,
}

impl TestHarness {
    fn new() -> Result<Self> {
        let runtime_registry = RuntimeRegistry::load(temp_runtime_registry())?;
        let runtime_host = RuntimeHost::new(
            runtime_registry,
            RuntimeManager::new(Arc::new(LocalProvider::new(Arc::new(FakeRuntimeLauncher)))),
        );
        let heartbeat_tracker = HeartbeatTracker::new();
        let token_store = RuntimeTokenStore::default();
        let app = build_router(AppState {
            runtime_host: runtime_host.clone(),
            heartbeat_tracker,
            token_store,
        });

        Ok(Self { app, runtime_host })
    }

    async fn create_runtime(&self, name: &str) -> Result<RuntimeDescriptor> {
        self.runtime_host
            .create(CreateRuntimeSpec {
                provider: fireline_conductor::runtime::RuntimeProviderRequest::Local,
                host: "127.0.0.1".parse::<IpAddr>()?,
                port: 0,
                name: name.to_string(),
                agent_command: vec!["/bin/echo".to_string(), "ok".to_string()],
                state_stream: Some(format!("state-{name}")),
                stream_storage: None,
                peer_directory_path: None,
                topology: TopologySpec::default(),
            })
            .await
    }

    async fn issue_runtime_token(&self, runtime_key: &str) -> Result<String> {
        let response = self
            .post_json(
                "/v1/auth/runtime-token",
                json!({
                    "runtimeKey": runtime_key,
                    "scope": "runtime.write",
                }),
                None,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK);

        let payload: Value = read_json(response).await?;
        Ok(payload["token"]
            .as_str()
            .expect("token response should include token")
            .to_string())
    }

    async fn get_runtime(&self, runtime_key: &str) -> Result<RuntimeDescriptor> {
        let response = self
            .request::<Value>(
                Method::GET,
                &format!("/v1/runtimes/{runtime_key}"),
                None,
                None,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        read_json(response).await
    }

    async fn post_json<T: Serialize>(
        &self,
        path: &str,
        body: T,
        bearer_token: Option<&str>,
    ) -> Result<axum::http::Response<Body>> {
        self.request(Method::POST, path, Some(&body), bearer_token)
            .await
    }

    async fn request<T: Serialize + ?Sized>(
        &self,
        method: Method,
        path: &str,
        body: Option<&T>,
        bearer_token: Option<&str>,
    ) -> Result<axum::http::Response<Body>> {
        let mut builder = axum::http::Request::builder().method(method).uri(path);
        if let Some(token) = bearer_token {
            builder = builder.header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"));
        }

        let request = if let Some(payload) = body {
            builder
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(payload)?))?
        } else {
            builder.body(Body::empty())?
        };

        Ok(self.app.clone().oneshot(request).await?)
    }
}

struct FakeRuntimeLauncher;

#[async_trait]
impl LocalRuntimeLauncher for FakeRuntimeLauncher {
    async fn start_local_runtime(
        &self,
        spec: CreateRuntimeSpec,
        _runtime_key: String,
        _node_id: String,
    ) -> Result<RuntimeLaunch> {
        Ok(RuntimeLaunch {
            status: RuntimeStatus::Starting,
            runtime_id: format!("fireline:{}:fake", spec.name),
            provider_instance_id: PRELAUNCH_PROVIDER_INSTANCE_ID.to_string(),
            acp: Endpoint::new(PRELAUNCH_ACP_URL),
            state: Endpoint::new(PRELAUNCH_STATE_STREAM_URL),
            helper_api_base_url: Some(PRELAUNCH_HELPER_API_BASE_URL.to_string()),
            runtime: Box::new(FakeManagedRuntime),
        })
    }
}

struct FakeManagedRuntime;

#[async_trait]
impl ManagedRuntime for FakeManagedRuntime {
    async fn shutdown(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}

fn registration_for(
    runtime: &RuntimeDescriptor,
    provider_instance_id: &str,
    advertised_acp_url: &str,
    advertised_state_stream_url: &str,
    helper_api_base_url: Option<&str>,
) -> RuntimeRegistration {
    RuntimeRegistration {
        runtime_id: runtime.runtime_id.clone(),
        node_id: runtime.node_id.clone(),
        provider: RuntimeProviderKind::Local,
        provider_instance_id: provider_instance_id.to_string(),
        advertised_acp_url: advertised_acp_url.to_string(),
        advertised_state_stream_url: advertised_state_stream_url.to_string(),
        helper_api_base_url: helper_api_base_url.map(ToString::to_string),
    }
}

async fn read_json<T: DeserializeOwned>(response: axum::http::Response<Body>) -> Result<T> {
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn temp_runtime_registry() -> PathBuf {
    std::env::temp_dir().join(format!(
        "fireline-control-plane-auth-{}.toml",
        Uuid::new_v4()
    ))
}
