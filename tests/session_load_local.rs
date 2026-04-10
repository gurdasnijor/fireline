use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_client_protocol::{
    ContentBlock, ContentChunk, InitializeRequest, LoadSessionRequest, NewSessionRequest,
    PromptRequest, ProtocolVersion, SessionNotification, SessionUpdate,
};
use agent_client_protocol_test::testy::TestyCommand;
use anyhow::Result;
use durable_streams::{Client as DsClient, Offset};
use fireline::bootstrap::{BootstrapConfig, start};
use fireline_conductor::topology::TopologySpec;
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::Arc;
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

        let (write, read) = StreamExt::split(ws);

        let outgoing = SinkExt::with(
            SinkExt::sink_map_err(write, std::io::Error::other),
            |line: String| async move {
                Ok::<_, std::io::Error>(tokio_tungstenite::tungstenite::Message::Text(line.into()))
            },
        );

        let incoming = StreamExt::filter_map(read, |msg| async move {
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
    PathBuf::from(env!("CARGO_BIN_EXE_fireline-testy"))
        .display()
        .to_string()
}

fn resumable_testy_bin() -> String {
    PathBuf::from(env!("CARGO_BIN_EXE_fireline-testy-load"))
        .display()
        .to_string()
}

fn temp_peer_directory() -> PathBuf {
    std::env::temp_dir().join(format!("fireline-peers-{}.toml", Uuid::new_v4()))
}

fn temp_stream_data_dir() -> PathBuf {
    std::env::temp_dir().join(format!("fireline-streams-{}", Uuid::new_v4()))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[tokio::test]
async fn session_load_returns_explicit_non_resumable_error_with_durable_record() -> Result<()> {
    let handle = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "session-load-live".to_string(),
        runtime_key: format!("runtime:{}", Uuid::new_v4()),
        node_id: "node:test-session-load".to_string(),
        agent_command: vec![testy_bin()],
        state_stream: None,
        external_stream_base_url: None,
        advertised_acp_url: None,
        stream_storage: None,
        peer_directory_path: temp_peer_directory(),
        topology: TopologySpec::default(),
    })
    .await?;

    let cwd = repo_root();
    let session_id = create_session(&handle.acp_url, &cwd).await?;
    wait_for_session_row(&handle.state_stream_url, &session_id).await?;
    let load_result = load_session(&handle.acp_url, &session_id, &cwd).await?;

    let error =
        load_result.expect_err("load should fail while downstream loadSession is unsupported");
    assert_eq!(error.message, "session_not_resumable");
    assert_eq!(i32::from(error.code), -32050);

    let fireline = error
        .data
        .as_ref()
        .and_then(|data| data.get("_meta"))
        .and_then(|meta| meta.get("fireline"))
        .expect("expected fireline metadata in error data");

    assert_eq!(
        fireline.get("error").and_then(Value::as_str),
        Some("session_not_resumable")
    );
    assert_eq!(
        fireline.get("reason").and_then(Value::as_str),
        Some("downstream_load_session_unsupported")
    );
    assert_eq!(
        fireline
            .get("sessionRecord")
            .and_then(|record| record.get("sessionId"))
            .and_then(Value::as_str),
        Some(session_id.as_str())
    );
    assert_eq!(
        fireline
            .get("sessionRecord")
            .and_then(|record| record.get("supportsLoadSession"))
            .and_then(Value::as_bool),
        Some(false)
    );

    handle.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn session_load_replays_catalog_after_restart_and_returns_same_durable_record() -> Result<()>
{
    let runtime_key = format!("runtime:{}", Uuid::new_v4());
    let state_stream = format!("fireline-session-load-{}", Uuid::new_v4());
    let peer_directory_path = temp_peer_directory();
    let cwd = repo_root();
    let stream_data_dir = temp_stream_data_dir();

    std::fs::create_dir_all(&stream_data_dir)?;

    let handle = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "session-load-restart".to_string(),
        runtime_key: runtime_key.clone(),
        node_id: "node:test-session-load".to_string(),
        agent_command: vec![testy_bin()],
        state_stream: Some(state_stream.clone()),
        external_stream_base_url: None,
        advertised_acp_url: None,
        stream_storage: Some(fireline::stream_host::StreamStorageConfig::file_durable(
            stream_data_dir.clone(),
        )),
        peer_directory_path: peer_directory_path.clone(),
        topology: TopologySpec::default(),
    })
    .await?;

    let session_id = create_session(&handle.acp_url, &cwd).await?;
    wait_for_session_row(&handle.state_stream_url, &session_id).await?;
    handle.shutdown().await?;

    let restarted = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "session-load-restart".to_string(),
        runtime_key: runtime_key.clone(),
        node_id: "node:test-session-load".to_string(),
        agent_command: vec![testy_bin()],
        state_stream: Some(state_stream),
        external_stream_base_url: None,
        advertised_acp_url: None,
        stream_storage: Some(fireline::stream_host::StreamStorageConfig::file_durable(
            stream_data_dir,
        )),
        peer_directory_path,
        topology: TopologySpec::default(),
    })
    .await?;

    let load_result = load_session(&restarted.acp_url, &session_id, &cwd).await?;
    let error = load_result
        .expect_err("replayed session should still be known and explicitly non-resumable");

    let fireline = error
        .data
        .as_ref()
        .and_then(|data| data.get("_meta"))
        .and_then(|meta| meta.get("fireline"))
        .expect("expected fireline metadata in error data");

    assert_eq!(
        fireline
            .get("sessionRecord")
            .and_then(|record| record.get("sessionId"))
            .and_then(Value::as_str),
        Some(session_id.as_str())
    );
    assert_eq!(
        fireline
            .get("sessionRecord")
            .and_then(|record| record.get("runtimeKey"))
            .and_then(Value::as_str),
        Some(runtime_key.as_str())
    );

    restarted.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn session_load_reattaches_against_runtime_owned_terminal_when_agent_supports_it()
-> Result<()> {
    let handle = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "session-load-resumable".to_string(),
        runtime_key: format!("runtime:{}", Uuid::new_v4()),
        node_id: "node:test-session-load".to_string(),
        agent_command: vec![resumable_testy_bin()],
        state_stream: None,
        external_stream_base_url: None,
        advertised_acp_url: None,
        stream_storage: None,
        peer_directory_path: temp_peer_directory(),
        topology: TopologySpec::default(),
    })
    .await?;

    let cwd = repo_root();
    let session_id = create_session(&handle.acp_url, &cwd).await?;
    wait_for_session_row(&handle.state_stream_url, &session_id).await?;

    let response = load_session_and_prompt(
        &handle.acp_url,
        &session_id,
        &cwd,
        &TestyCommand::Echo {
            message: "reattach succeeded".to_string(),
        }
        .to_prompt(),
    )
    .await?;

    assert_eq!(response, "reattach succeeded");

    handle.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn session_load_after_restart_forwards_and_surfaces_downstream_session_not_found()
-> Result<()> {
    let runtime_key = format!("runtime:{}", Uuid::new_v4());
    let state_stream = format!("fireline-session-load-resumable-{}", Uuid::new_v4());
    let peer_directory_path = temp_peer_directory();
    let cwd = repo_root();
    let stream_data_dir = temp_stream_data_dir();

    std::fs::create_dir_all(&stream_data_dir)?;

    let handle = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "session-load-resumable-restart".to_string(),
        runtime_key: runtime_key.clone(),
        node_id: "node:test-session-load".to_string(),
        agent_command: vec![resumable_testy_bin()],
        state_stream: Some(state_stream.clone()),
        external_stream_base_url: None,
        advertised_acp_url: None,
        stream_storage: Some(fireline::stream_host::StreamStorageConfig::file_durable(
            stream_data_dir.clone(),
        )),
        peer_directory_path: peer_directory_path.clone(),
        topology: TopologySpec::default(),
    })
    .await?;

    let session_id = create_session(&handle.acp_url, &cwd).await?;
    wait_for_session_row(&handle.state_stream_url, &session_id).await?;
    handle.shutdown().await?;

    let restarted = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "session-load-resumable-restart".to_string(),
        runtime_key,
        node_id: "node:test-session-load".to_string(),
        agent_command: vec![resumable_testy_bin()],
        state_stream: Some(state_stream),
        external_stream_base_url: None,
        advertised_acp_url: None,
        stream_storage: Some(fireline::stream_host::StreamStorageConfig::file_durable(
            stream_data_dir,
        )),
        peer_directory_path,
        topology: TopologySpec::default(),
    })
    .await?;

    let load_result = load_session(&restarted.acp_url, &session_id, &cwd).await?;
    let error = load_result.expect_err(
        "restarted runtime should forward to downstream loadSession and surface session_not_found",
    );

    assert_eq!(error.message, "session_not_found");
    assert_eq!(i32::from(error.code), -32061);
    assert_eq!(
        error
            .data
            .as_ref()
            .and_then(|data| data.get("sessionId"))
            .and_then(Value::as_str),
        Some(session_id.as_str())
    );

    restarted.shutdown().await?;
    Ok(())
}

async fn create_session(acp_url: &str, cwd: &Path) -> Result<String> {
    let cwd = cwd.to_path_buf();

    sacp::Client
        .builder()
        .connect_with(
            WebSocketTransport {
                url: acp_url.to_string(),
            },
            move |cx: sacp::ConnectionTo<sacp::Agent>| {
                let cwd = cwd.clone();
                async move {
                    let _ = cx
                        .send_request(InitializeRequest::new(ProtocolVersion::LATEST))
                        .block_task()
                        .await?;

                    let session = cx
                        .send_request(NewSessionRequest::new(cwd))
                        .block_task()
                        .await?;

                    Ok(session.session_id.to_string())
                }
            },
        )
        .await
        .map_err(anyhow::Error::from)
}

async fn load_session(
    acp_url: &str,
    session_id: &str,
    cwd: &Path,
) -> Result<Result<agent_client_protocol::LoadSessionResponse, sacp::Error>> {
    let cwd = cwd.to_path_buf();
    let session_id = session_id.to_string();

    sacp::Client
        .builder()
        .connect_with(
            WebSocketTransport {
                url: acp_url.to_string(),
            },
            move |cx: sacp::ConnectionTo<sacp::Agent>| {
                let cwd = cwd.clone();
                let session_id = session_id.clone();
                async move {
                    let _ = cx
                        .send_request(InitializeRequest::new(ProtocolVersion::LATEST))
                        .block_task()
                        .await?;

                    Ok(cx
                        .send_request(LoadSessionRequest::new(session_id, cwd))
                        .block_task()
                        .await)
                }
            },
        )
        .await
        .map_err(anyhow::Error::from)
}

async fn load_session_and_prompt(
    acp_url: &str,
    session_id: &str,
    cwd: &Path,
    prompt_text: &str,
) -> Result<String> {
    let cwd = cwd.to_path_buf();
    let session_id = session_id.to_string();
    let prompt_text = prompt_text.to_string();
    let response_text = Arc::new(tokio::sync::Mutex::new(String::new()));

    sacp::Client
        .builder()
        .on_receive_notification(
            {
                let response_text = response_text.clone();
                async move |notification: SessionNotification, _cx| {
                    if let SessionUpdate::AgentMessageChunk(ContentChunk {
                        content: ContentBlock::Text(text),
                        ..
                    }) = notification.update
                    {
                        response_text.lock().await.push_str(&text.text);
                    }
                    Ok(())
                }
            },
            sacp::on_receive_notification!(),
        )
        .connect_with(
            WebSocketTransport {
                url: acp_url.to_string(),
            },
            move |cx: sacp::ConnectionTo<sacp::Agent>| {
                let cwd = cwd.clone();
                let session_id = session_id.clone();
                let prompt_text = prompt_text.clone();
                async move {
                    let _ = cx
                        .send_request(InitializeRequest::new(ProtocolVersion::LATEST))
                        .block_task()
                        .await?;

                    let _ = cx
                        .send_request(LoadSessionRequest::new(session_id.clone(), cwd))
                        .block_task()
                        .await?;

                    let _ = cx
                        .send_request(PromptRequest::new(session_id, vec![prompt_text.into()]))
                        .block_task()
                        .await?;

                    Ok(())
                }
            },
        )
        .await
        .map_err(anyhow::Error::from)?;

    Ok(response_text.lock().await.clone())
}

async fn wait_for_session_row(state_stream_url: &str, session_id: &str) -> Result<()> {
    let client = DsClient::new();
    let stream = client.stream(state_stream_url);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);

    loop {
        let mut reader = stream.read().offset(Offset::Beginning).build()?;
        let mut body = String::new();

        while let Some(chunk) = reader.next_chunk().await? {
            body.push_str(std::str::from_utf8(&chunk.data)?);
            if chunk.up_to_date {
                break;
            }
        }

        if body.contains("\"type\":\"session\"") && body.contains(session_id) {
            return Ok(());
        }

        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for durable session row {session_id}");
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
