//! Process bootstrap.
//!
//! `bootstrap::start(config)` brings up everything the Fireline
//! binary needs in one process:
//!
//! 1. Build the embedded durable-streams Router via
//!    [`crate::stream_host::build_stream_router`]
//! 2. Build the `durable-streams::Producer` that the state writer
//!    will append to (HTTP client pointed at our own listener)
//! 3. Compose the component list (`PeerComponent`, future components)
//! 4. Build the axum Router and `.merge()` in the stream Router so
//!    `/healthz`, `/v1/stream/{name}`, `/acp`, and `/api/v1/files/*`
//!    all live on a single listener (Option A embedding)
//! 5. Bind the listener on `config.host:config.port` and serve
//! 6. Return a handle that can be `.shutdown()`'d gracefully

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use anyhow::{Context, Result};
use axum::Router;
use durable_streams::{Client as DurableStreamsClient, CreateOptions, DurableStream, Producer};
use fireline_components::LocalPeerDirectory;
use fireline_conductor::topology::{TopologyRegistry, TopologySpec};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub conductor_name: String,
    pub runtime_key: String,
    pub node_id: String,
    pub runtime_id: String,
    pub state_producer: Producer,
    pub peer_directory_path: PathBuf,
    pub session_index: crate::session_index::SessionIndex,
    pub active_turn_index: crate::active_turn_index::ActiveTurnIndex,
    pub shared_terminal: fireline_conductor::shared_terminal::SharedTerminal,
    pub topology_registry: TopologyRegistry,
    pub topology: TopologySpec,
}

#[derive(Debug, Clone)]
pub struct BootstrapConfig {
    pub host: IpAddr,
    pub port: u16,
    pub name: String,
    pub runtime_key: String,
    pub node_id: String,
    pub agent_command: Vec<String>,
    pub state_stream: Option<String>,
    pub stream_storage: Option<fireline_conductor::runtime::StreamStorageConfig>,
    pub peer_directory_path: PathBuf,
    pub topology: TopologySpec,
}

pub struct BootstrapHandle {
    pub runtime_id: String,
    pub state_stream: String,
    pub health_url: String,
    pub acp_url: String,
    pub state_stream_url: String,
    runtime_name: String,
    runtime_created_at: i64,
    state_producer: Producer,
    peer_directory_path: PathBuf,
    runtime_materializer_task: crate::runtime_materializer::RuntimeMaterializerTask,
    shared_terminal: fireline_conductor::shared_terminal::SharedTerminal,
    shutdown_tx: Option<oneshot::Sender<()>>,
    server_task: JoinHandle<Result<()>>,
}

impl BootstrapHandle {
    pub async fn shutdown(mut self) -> Result<()> {
        LocalPeerDirectory::load(&self.peer_directory_path)?
            .unregister(&self.runtime_id)
            .context("unregister peer runtime")?;

        fireline_conductor::trace::emit_runtime_instance_stopped(
            &self.state_producer,
            &self.runtime_id,
            &self.runtime_name,
            self.runtime_created_at,
        );

        self.runtime_materializer_task.abort();

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
    let runtime_uuid = Uuid::new_v4();
    let runtime_key = config.runtime_key;
    let runtime_id = format!("fireline:{}:{runtime_uuid}", config.name);
    let runtime_created_at = chrono_like_now_ms();
    let state_stream = config
        .state_stream
        .unwrap_or_else(|| format!("fireline-state-{runtime_uuid}"));

    let listener = TcpListener::bind(SocketAddr::new(config.host, config.port)).await?;
    let local_addr = listener
        .local_addr()
        .context("resolve bound listener address")?;
    let connect_host_name = connect_host(local_addr.ip());
    let health_url = format!("http://{connect_host_name}:{}/healthz", local_addr.port());
    let acp_url = format!("ws://{connect_host_name}:{}/acp", local_addr.port());
    let state_stream_url = format!(
        "http://{connect_host_name}:{}/v1/stream/{state_stream}",
        local_addr.port()
    );
    let stream_base_url = format!("http://{connect_host_name}:{}/v1/stream", local_addr.port());
    let runtime_name = config.name.clone();

    let stream_client = DurableStreamsClient::new();
    let mut state_stream_handle = stream_client.stream(&state_stream_url);
    state_stream_handle.set_content_type("application/json");
    let state_producer = state_stream_handle
        .producer(format!("state-writer-{runtime_uuid}"))
        .content_type("application/json")
        .build();
    let node_id = config.node_id;
    let peer_directory_path = config.peer_directory_path;
    let directory = LocalPeerDirectory::load(&peer_directory_path)?;
    let session_index = crate::session_index::SessionIndex::new();
    let active_turn_index = crate::active_turn_index::ActiveTurnIndex::new();
    let runtime_materializer = crate::runtime_materializer::RuntimeMaterializer::new(vec![
        std::sync::Arc::new(session_index.clone()),
        std::sync::Arc::new(active_turn_index.clone()),
    ]);
    let shared_terminal =
        fireline_conductor::shared_terminal::SharedTerminal::spawn(config.agent_command).await?;
    let topology_registry =
        crate::topology::build_runtime_topology_registry(crate::topology::ComponentContext {
            runtime_key: runtime_key.clone(),
            runtime_id: runtime_id.clone(),
            node_id: node_id.clone(),
            stream_base_url: stream_base_url.clone(),
            peer_registry: std::sync::Arc::new(directory.clone()),
            active_turn_lookup: std::sync::Arc::new(active_turn_index.clone()),
            child_session_edge_sink: std::sync::Arc::new(
                crate::child_session_edge::ChildSessionEdgeWriter::new(state_producer.clone()),
            ),
        });

    let app_state = AppState {
        conductor_name: runtime_name.clone(),
        runtime_key: runtime_key.clone(),
        node_id: node_id.clone(),
        runtime_id: runtime_id.clone(),
        state_producer: state_producer.clone(),
        peer_directory_path: peer_directory_path.clone(),
        session_index: session_index.clone(),
        active_turn_index: active_turn_index.clone(),
        shared_terminal: shared_terminal.clone(),
        topology_registry,
        topology: config.topology.clone(),
    };

    let app = Router::new()
        .merge(crate::routes::acp::router(app_state))
        .merge(crate::stream_host::build_stream_router(
            config.stream_storage.as_ref(),
        )?);

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .map_err(anyhow::Error::from)
    });

    crate::topology::ensure_named_streams(
        &stream_base_url,
        &crate::topology::audit_stream_names(&config.topology)?,
    )
    .await?;
    ensure_stream_exists(&state_stream_handle).await?;
    let runtime_materializer_task = runtime_materializer.connect(state_stream_url.clone());
    runtime_materializer_task.preload().await?;
    fireline_conductor::trace::emit_runtime_instance_started(
        &state_producer,
        &runtime_id,
        &runtime_name,
        runtime_created_at,
    );
    directory.register(fireline_components::directory::Peer {
        runtime_id: runtime_id.clone(),
        agent_name: runtime_name.clone(),
        acp_url: acp_url.clone(),
        state_stream_url: Some(state_stream_url.clone()),
        registered_at_ms: runtime_created_at,
    })?;

    Ok(BootstrapHandle {
        runtime_id,
        state_stream,
        health_url,
        acp_url,
        state_stream_url,
        runtime_name,
        runtime_created_at,
        state_producer,
        peer_directory_path,
        runtime_materializer_task,
        shared_terminal,
        shutdown_tx: Some(shutdown_tx),
        server_task,
    })
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

fn chrono_like_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}
