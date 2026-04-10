use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use agent_client_protocol::{InitializeRequest, ProtocolVersion};
use anyhow::Result;
use axum::Router;
use durable_streams::{Client as DsClient, Offset};
use fireline::bootstrap::{BootstrapConfig, start};
use fireline_conductor::topology::TopologySpec;
use tokio::sync::oneshot;
use uuid::Uuid;

struct WebSocketTransport {
    url: String,
}

impl sacp::ConnectTo<sacp::Client> for WebSocketTransport {
    async fn connect_to(
        self,
        client: impl sacp::ConnectTo<sacp::Agent>,
    ) -> Result<(), sacp::Error> {
        let (ws, _) = tokio_tungstenite::connect_async(self.url.as_str())
            .await
            .map_err(|e| sacp::util::internal_error(format!("WebSocket connect: {e}")))?;

        let (write, read) = futures::StreamExt::split(ws);

        let outgoing = futures::SinkExt::with(
            futures::SinkExt::sink_map_err(write, std::io::Error::other),
            |line: String| async move {
                Ok::<_, std::io::Error>(tokio_tungstenite::tungstenite::Message::Text(line.into()))
            },
        );

        let incoming = futures::StreamExt::filter_map(read, |msg| async move {
            match msg {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                    let line = text.trim().to_string();
                    if line.is_empty() {
                        None
                    } else {
                        Some(Ok(line))
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Binary(bytes)) => {
                    String::from_utf8(bytes.to_vec()).ok().and_then(|text| {
                        let line = text.trim().to_string();
                        if line.is_empty() {
                            None
                        } else {
                            Some(Ok(line))
                        }
                    })
                }
                Ok(_) => None,
                Err(err) => Some(Err(std::io::Error::other(err))),
            }
        });

        sacp::ConnectTo::<sacp::Client>::connect_to(sacp::Lines::new(outgoing, incoming), client)
            .await
    }
}

fn testy_bin() -> String {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_fireline-testy"))
        .display()
        .to_string()
}

fn temp_peer_directory() -> PathBuf {
    std::env::temp_dir().join(format!("fireline-peers-{}.toml", Uuid::new_v4()))
}

struct ExternalStreamServer {
    base_url: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl ExternalStreamServer {
    async fn start() -> Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let app: Router = fireline::stream_host::build_stream_router(None)?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .map_err(anyhow::Error::from)
        });

        Ok(Self {
            base_url: format!("http://127.0.0.1:{}/v1/stream", addr.port()),
            shutdown_tx: Some(shutdown_tx),
            task,
        })
    }

    async fn shutdown(mut self) -> Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        self.task.await??;
        Ok(())
    }
}

#[tokio::test]
async fn hosted_runtime_serves_acp_and_emits_state_events() -> Result<()> {
    let handle = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "hosted-test".to_string(),
        runtime_key: format!("runtime:{}", Uuid::new_v4()),
        node_id: "node:test-hosted".to_string(),
        agent_command: vec![testy_bin()],
        state_stream: None,
        external_stream_base_url: None,
        advertised_acp_url: None,
        stream_storage: None,
        peer_directory_path: temp_peer_directory(),
        topology: TopologySpec::default(),
    })
    .await?;

    let response = yopo::prompt(
        WebSocketTransport {
            url: handle.acp_url.clone(),
        },
        "hello from hosted runtime",
    )
    .await?;

    assert_eq!(
        response, "Hello, world!",
        "fireline-testy should respond through the SDK test agent"
    );

    let client = DsClient::new();
    let stream = client.stream(&handle.state_stream_url);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut body = String::new();

    loop {
        body.clear();

        let mut reader = stream.read().offset(Offset::Beginning).build()?;
        while let Some(chunk) = reader.next_chunk().await? {
            body.push_str(std::str::from_utf8(&chunk.data)?);
            if chunk.up_to_date {
                break;
            }
        }

        if body.contains("\"type\":\"prompt_turn\"") {
            break;
        }

        if tokio::time::Instant::now() >= deadline {
            break;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert!(
        body.contains("\"type\":\"runtime_instance\""),
        "state stream should contain runtime instance rows: {body}"
    );
    assert!(
        body.contains("\"type\":\"prompt_turn\""),
        "state stream should contain prompt turns: {body}"
    );
    assert!(
        body.contains("\"type\":\"session\""),
        "state stream should contain session rows: {body}"
    );

    handle.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn hosted_runtime_rejects_concurrent_attachment_and_recovers_after_disconnect() -> Result<()>
{
    let handle = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "hosted-busy-test".to_string(),
        runtime_key: format!("runtime:{}", Uuid::new_v4()),
        node_id: "node:test-hosted".to_string(),
        agent_command: vec![testy_bin()],
        state_stream: None,
        external_stream_base_url: None,
        advertised_acp_url: None,
        stream_storage: None,
        peer_directory_path: temp_peer_directory(),
        topology: TopologySpec::default(),
    })
    .await?;

    let acp_url = handle.acp_url.clone();
    let held_connection = tokio::spawn(async move {
        sacp::Client
            .builder()
            .connect_with(
                WebSocketTransport {
                    url: acp_url.clone(),
                },
                move |cx: sacp::ConnectionTo<sacp::Agent>| async move {
                    let _ = cx
                        .send_request(InitializeRequest::new(ProtocolVersion::LATEST))
                        .block_task()
                        .await?;
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    Ok::<(), sacp::Error>(())
                },
            )
            .await
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    match tokio_tungstenite::connect_async(handle.acp_url.as_str()).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
        }
        other => panic!("expected runtime_busy conflict, got {other:?}"),
    }

    held_connection.await??;

    let response = yopo::prompt(
        WebSocketTransport {
            url: handle.acp_url.clone(),
        },
        "hello after disconnect",
    )
    .await?;

    assert_eq!(response, "Hello, world!");

    handle.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn hosted_runtime_can_target_external_durable_streams() -> Result<()> {
    let external_stream_server = ExternalStreamServer::start().await?;

    let handle = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "hosted-external-stream".to_string(),
        runtime_key: format!("runtime:{}", Uuid::new_v4()),
        node_id: "node:test-hosted".to_string(),
        agent_command: vec![testy_bin()],
        state_stream: Some(format!("fireline-external-{}", Uuid::new_v4())),
        external_stream_base_url: Some(external_stream_server.base_url.clone()),
        advertised_acp_url: None,
        stream_storage: None,
        peer_directory_path: temp_peer_directory(),
        topology: TopologySpec::default(),
    })
    .await?;

    assert!(handle
        .state_stream_url
        .starts_with(&external_stream_server.base_url));

    let response = yopo::prompt(
        WebSocketTransport {
            url: handle.acp_url.clone(),
        },
        "hello against external state plane",
    )
    .await;

    assert_eq!(response?, "Hello, world!");

    let client = DsClient::new();
    let stream = client.stream(&handle.state_stream_url);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut body = String::new();

    loop {
        body.clear();

        let mut reader = stream.read().offset(Offset::Beginning).build()?;
        while let Some(chunk) = reader.next_chunk().await? {
            body.push_str(std::str::from_utf8(&chunk.data)?);
            if chunk.up_to_date {
                break;
            }
        }

        if body.contains("\"type\":\"prompt_turn\"") {
            break;
        }

        if tokio::time::Instant::now() >= deadline {
            break;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert!(
        body.contains("\"type\":\"runtime_instance\""),
        "external state stream should contain runtime instance rows: {body}"
    );
    assert!(
        body.contains("\"type\":\"prompt_turn\""),
        "external state stream should contain prompt turns: {body}"
    );

    handle.shutdown().await?;
    external_stream_server.shutdown().await?;
    Ok(())
}
