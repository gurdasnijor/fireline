//! Process bootstrap.
//!
//! `bootstrap::start(config)` brings up everything the Fireline
//! binary needs in one process:
//!
//! 1. Connect to the external durable-streams service supplied under
//!    `config.durable_streams_url`
//! 2. Build the `durable-streams::Producer` that the state writer
//!    will append to (HTTP client pointed at our own listener)
//! 3. Compose the component list (`PeerComponent`, future components)
//! 4. Build the axum Router so `/healthz` and `/acp` live on a single
//!    listener while state rows flow to the external stream service
//! 5. Bind the listener on `config.host:config.port` and serve
//! 6. Return a handle that can be `.shutdown()`'d gracefully

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use axum::routing::get;
use durable_streams::{Client as DurableStreamsClient, CreateOptions, DurableStream, Producer};
use fireline_harness::{
    AcpRouteState, ComponentContext, SharedTerminal, TopologySpec, audit_stream_names,
    build_host_topology_registry, emit_host_instance_started, emit_host_instance_stopped,
    emit_host_spec_persisted, ensure_named_streams,
};
use fireline_orchestration::load_coordinator::LoadCoordinatorComponent;
use fireline_resources::MountedResource;
use fireline_session::{
    ActiveTurnIndex, PersistedHostSpec, ProvisionSpec, SandboxProviderRequest, SessionIndex,
    StateMaterializer, StateMaterializerTask,
};
use fireline_tools::{
    DEFAULT_TENANT_ID, DeploymentDiscoveryEvent, PeerRegistry, StreamDeploymentPeerRegistry,
    deployment_stream_url,
};
use serde_json::{Map, Value};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use uuid::Uuid;

const HOST_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct BootstrapConfig {
    pub host: IpAddr,
    pub port: u16,
    pub name: String,
    pub host_key: String,
    pub node_id: String,
    pub agent_command: Vec<String>,
    pub mounted_resources: Vec<MountedResource>,
    pub state_stream: Option<String>,
    pub durable_streams_url: String,
    pub peer_directory_path: PathBuf,
    pub control_plane_url: Option<String>,
    pub topology: TopologySpec,
}

pub struct BootstrapHandle {
    pub host_id: String,
    pub state_stream: String,
    pub health_url: String,
    pub acp_url: String,
    pub state_stream_url: String,
    host_key: String,
    host_name: String,
    host_created_at: i64,
    state_producer: Producer,
    deployment_producer: Producer,
    state_materializer_task: StateMaterializerTask,
    deployment_heartbeat_task: JoinHandle<()>,
    shared_terminal: SharedTerminal,
    shutdown_tx: Option<oneshot::Sender<()>>,
    server_task: JoinHandle<Result<()>>,
}

impl BootstrapHandle {
    pub async fn shutdown(mut self) -> Result<()> {
        self.deployment_heartbeat_task.abort();
        let _ = self.deployment_heartbeat_task.await;

        emit_deployment_event(
            &self.deployment_producer,
            &DeploymentDiscoveryEvent::HostStopped {
                host_id: self.host_id.clone(),
                host_key: self.host_key.clone(),
                stopped_at_ms: chrono_like_now_ms(),
            },
        )
        .await
        .context("flush host_stopped on shutdown")?;

        emit_deployment_event(
            &self.deployment_producer,
            &DeploymentDiscoveryEvent::HostDeregistered {
                host_id: self.host_id.clone(),
                reason: "graceful_shutdown".to_string(),
                deregistered_at_ms: chrono_like_now_ms(),
            },
        )
        .await
        .context("flush host_deregistered on shutdown")?;

        emit_host_instance_stopped(
            &self.state_producer,
            &self.host_id,
            &self.host_name,
            self.host_created_at,
        )
        .await
        .context("flush host_instance_stopped on shutdown")?;

        self.state_materializer_task.abort();

        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        self.shared_terminal.shutdown().await?;

        self.server_task
            .await
            .context("join server task")?
            .context("serve fireline")
    }
}

pub async fn start(config: BootstrapConfig) -> Result<BootstrapHandle> {
    let host_uuid = Uuid::new_v4();
    let host_key = config.host_key;
    let host_id = format!("fireline:{}:{host_uuid}", config.name);
    let host_created_at = chrono_like_now_ms();
    let state_stream = config
        .state_stream
        .unwrap_or_else(|| format!("fireline-state-{host_uuid}"));

    let listener = TcpListener::bind(SocketAddr::new(config.host, config.port)).await?;
    let local_addr = listener
        .local_addr()
        .context("resolve bound listener address")?;
    let connect_host_name = connect_host(local_addr.ip());
    let health_url = format!("http://{connect_host_name}:{}/healthz", local_addr.port());
    let acp_url = format!("ws://{connect_host_name}:{}/acp", local_addr.port());
    let stream_base_url = config.durable_streams_url.trim_end_matches('/').to_string();
    let state_stream_url = format!("{stream_base_url}/{state_stream}");
    let host_stream_url = deployment_stream_url(&stream_base_url, DEFAULT_TENANT_ID);
    let host_name = config.name.clone();

    let stream_client = DurableStreamsClient::new();
    let mut state_stream_handle = stream_client.stream(&state_stream_url);
    state_stream_handle.set_content_type("application/json");
    let state_producer = state_stream_handle
        .producer(format!("state-writer-{host_uuid}"))
        .content_type("application/json")
        .build();
    let mut host_stream_handle = stream_client.stream(&host_stream_url);
    host_stream_handle.set_content_type("application/json");
    let deployment_producer = host_stream_handle
        .producer(format!("deployment-discovery-{host_uuid}"))
        .content_type("application/json")
        .build();
    let node_id = config.node_id;
    let session_index = SessionIndex::new();
    let active_turn_index = ActiveTurnIndex::new();
    let state_materializer = StateMaterializer::new(vec![
        std::sync::Arc::new(session_index.clone()),
        std::sync::Arc::new(active_turn_index.clone()),
    ]);
    // Keep a clone of the agent command around so we can thread it into
    // the `host_spec` envelope further down — SharedTerminal::spawn
    // consumes the original.
    let agent_command_for_spec = config.agent_command.clone();
    let shared_terminal = SharedTerminal::spawn(config.agent_command).await?;
    ensure_named_streams(&stream_base_url, &audit_stream_names(&config.topology)?).await?;
    ensure_stream_exists(&state_stream_handle).await?;
    ensure_stream_exists(&host_stream_handle).await?;
    let peer_registry: std::sync::Arc<dyn PeerRegistry> = std::sync::Arc::new(
        StreamDeploymentPeerRegistry::new(stream_base_url.clone(), DEFAULT_TENANT_ID),
    );
    let topology_registry = build_host_topology_registry(ComponentContext {
        host_key: host_key.clone(),
        host_id: host_id.clone(),
        node_id: node_id.clone(),
        stream_base_url: stream_base_url.clone(),
        state_stream_url: state_stream_url.clone(),
        state_producer: state_producer.clone(),
        peer_registry: peer_registry.clone(),
        active_turn_lookup: std::sync::Arc::new(active_turn_index.clone()),
        mounted_resources: config.mounted_resources.clone(),
    });

    let app_state = AcpRouteState {
        conductor_name: host_name.clone(),
        host_key: host_key.clone(),
        node_id: node_id.clone(),
        host_id: host_id.clone(),
        state_producer: state_producer.clone(),
        shared_terminal: shared_terminal.clone(),
        topology_registry,
        topology: config.topology.clone(),
        base_components_factory: std::sync::Arc::new({
            let session_index = session_index.clone();
            move || {
                vec![sacp::DynConnectTo::new(LoadCoordinatorComponent::new(
                    session_index.clone(),
                ))]
            }
        }),
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .merge(fireline_harness::routes_acp::router(app_state));

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .map_err(anyhow::Error::from)
    });

    // Emit stream events BEFORE the materializer preloads, so
    // the stream has content when the materializer subscribes.
    // Without this ordering, preload connects to an empty stream,
    // the worker finds nothing to replay, and exits — causing
    // "state materializer worker exited before preload completed."
    let persisted_spec = PersistedHostSpec::new(
        host_key.clone(),
        node_id.clone(),
        ProvisionSpec {
            host_key: Some(host_key.clone()),
            node_id: Some(node_id.clone()),
            provider: SandboxProviderRequest::Local,
            host: config.host,
            port: local_addr.port(),
            name: host_name.clone(),
            agent_command: agent_command_for_spec,
            durable_streams_url: stream_base_url.clone(),
            resources: Vec::new(),
            state_stream: Some(state_stream.clone()),
            stream_storage: None,
            peer_directory_path: None,
            topology: config.topology.clone(),
        },
    );
    emit_host_spec_persisted(&state_stream_url, &persisted_spec)
        .await
        .context("emit host_spec_persisted from direct-host bootstrap")?;

    emit_host_instance_started(&state_producer, &host_id, &host_name, host_created_at)
        .await
        .context("flush runtime_instance_started from bootstrap")?;

    let state_materializer_task = state_materializer.connect(state_stream_url.clone());
    state_materializer_task.preload().await?;
    emit_deployment_event(
        &deployment_producer,
        &DeploymentDiscoveryEvent::HostRegistered {
            host_id: host_id.clone(),
            acp_url: acp_url.clone(),
            state_stream_url: state_stream_url.clone(),
            capabilities: host_capabilities(),
            registered_at_ms: host_created_at,
            node_info: host_node_info(&node_id),
        },
    )
    .await
    .context("flush host_registered from bootstrap")?;
    emit_deployment_event(
        &deployment_producer,
        &DeploymentDiscoveryEvent::HostProvisioned {
            host_id: host_id.clone(),
            host_key: host_key.clone(),
            acp_url: acp_url.clone(),
            agent_name: host_name.clone(),
            provisioned_at_ms: host_created_at,
        },
    )
    .await
    .context("flush host_provisioned from bootstrap")?;
    let deployment_heartbeat_task = tokio::spawn(run_host_heartbeat_loop(
        deployment_producer.clone(),
        host_id.clone(),
    ));

    Ok(BootstrapHandle {
        host_id,
        state_stream,
        health_url,
        acp_url,
        state_stream_url,
        host_key,
        host_name,
        host_created_at,
        state_producer,
        deployment_producer,
        state_materializer_task,
        deployment_heartbeat_task,
        shared_terminal,
        shutdown_tx: Some(shutdown_tx),
        server_task,
    })
}

async fn healthz() -> &'static str {
    "ok"
}

fn connect_host(ip: IpAddr) -> String {
    if ip.is_unspecified() {
        match ip {
            IpAddr::V4(_) => "127.0.0.1".to_string(),
            IpAddr::V6(_) => "::1".to_string(),
        }
    } else {
        ip.to_string()
    }
}

async fn ensure_stream_exists(stream: &DurableStream) -> Result<()> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match stream
            .create_with(CreateOptions::new().content_type("application/json"))
            .await
        {
            Ok(_) => return Ok(()),
            Err(err) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(anyhow::Error::from(err)).context("create state stream");
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }
}

async fn emit_deployment_event(
    producer: &Producer,
    event: &DeploymentDiscoveryEvent,
) -> Result<()> {
    producer.append_json(event);
    producer
        .flush()
        .await
        .map_err(anyhow::Error::from)
        .context("flush deployment discovery event")
}

async fn run_host_heartbeat_loop(producer: Producer, host_id: String) {
    let mut interval = tokio::time::interval(HOST_HEARTBEAT_INTERVAL);
    loop {
        interval.tick().await;
        if let Err(error) = emit_deployment_event(
            &producer,
            &DeploymentDiscoveryEvent::HostHeartbeat {
                host_id: host_id.clone(),
                seen_at_ms: chrono_like_now_ms(),
                load_metrics: Map::new(),
                provisioned_host_count: 1,
            },
        )
        .await
        {
            tracing::warn!(?error, host_id, "flush host_heartbeat");
        }
    }
}

fn host_capabilities() -> Map<String, Value> {
    Map::from_iter([
        ("peerCalls".to_string(), Value::Bool(true)),
        ("sharedState".to_string(), Value::Bool(true)),
    ])
}

fn host_node_info(node_id: &str) -> Map<String, Value> {
    Map::from_iter([
        ("nodeId".to_string(), Value::String(node_id.to_string())),
        (
            "version".to_string(),
            Value::String(format!("fireline/{}", env!("CARGO_PKG_VERSION"))),
        ),
    ])
}

fn chrono_like_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::ZERO)
        .as_millis() as i64
}
