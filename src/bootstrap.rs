//! Process bootstrap.
//!
//! `bootstrap::start(config)` brings up everything the Fireline
//! binary needs in one process:
//!
//! 1. Build the embedded durable-streams Router via
//!    [`crate::stream_host::build_stream_router`]
//! 2. Build the `durable-streams::Producer` that the trace writer
//!    will append to (HTTP client pointed at our own listener)
//! 3. Compose the component list (`PeerComponent`, future components)
//! 4. Build the axum Router and `.merge()` in the stream Router so
//!    `/healthz`, `/v1/stream/{name}`, `/acp`, and `/api/v1/files/*`
//!    all live on a single listener (Option A embedding)
//! 5. Spawn the webhook subscriber if any webhooks are configured
//! 6. Bind the listener on `config.host:config.port` and serve
//! 7. Return a handle that can be `.shutdown()`'d gracefully

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use anyhow::{Context, Result};
use axum::Router;
use durable_streams::{Client as DurableStreamsClient, DurableStream, Producer};
use fireline_peer::Directory;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub conductor_name: String,
    pub agent_command: Vec<String>,
    pub runtime_id: String,
    pub trace_producer: Producer,
    pub peer_directory_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct BootstrapConfig {
    pub host: IpAddr,
    pub port: u16,
    pub name: String,
    pub agent_command: Vec<String>,
    pub state_stream: Option<String>,
}

pub struct BootstrapHandle {
    pub runtime_id: String,
    pub state_stream: String,
    pub acp_url: String,
    pub state_stream_url: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    server_task: JoinHandle<Result<()>>,
}

impl BootstrapHandle {
    pub async fn shutdown(mut self) -> Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        self.server_task
            .await
            .context("join server task")?
            .context("serve fireline")
    }
}

pub async fn start(config: BootstrapConfig) -> Result<BootstrapHandle> {
    let runtime_uuid = Uuid::new_v4();
    let runtime_id = format!("fireline:{}:{runtime_uuid}", config.name);
    let state_stream = config
        .state_stream
        .unwrap_or_else(|| format!("fireline-trace-{runtime_uuid}"));

    let listener = TcpListener::bind(SocketAddr::new(config.host, config.port)).await?;
    let local_addr = listener.local_addr().context("resolve bound listener address")?;
    let connect_host = connect_host(local_addr.ip());
    let acp_url = format!("ws://{connect_host}:{}/acp", local_addr.port());
    let state_stream_url = format!("http://{connect_host}:{}/v1/stream/{state_stream}", local_addr.port());

    let stream_client = DurableStreamsClient::new();
    let trace_stream = stream_client.stream(&state_stream_url);
    let trace_producer = trace_stream
        .producer(format!("trace-writer-{runtime_uuid}"))
        .build();

    let app_state = AppState {
        conductor_name: config.name,
        agent_command: config.agent_command,
        runtime_id: runtime_id.clone(),
        trace_producer,
        peer_directory_path: Directory::default_path()?,
    };

    let app = Router::new()
        .merge(crate::routes::acp::router(app_state))
        .merge(crate::stream_host::build_stream_router()?);

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .map_err(anyhow::Error::from)
    });

    ensure_stream_exists(&trace_stream).await?;

    Ok(BootstrapHandle {
        runtime_id,
        state_stream,
        acp_url,
        state_stream_url,
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
        match stream.create().await {
            Ok(_) => return Ok(()),
            Err(err) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(anyhow::Error::from(err)).context("create trace stream");
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }
}
