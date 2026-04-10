use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use agent_client_protocol_test::testy::TestyCommand;
use anyhow::Result;
use durable_streams::{Client as DsClient, Offset};
use fireline::bootstrap::{BootstrapConfig, start};
use futures::{SinkExt, StreamExt};
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

fn temp_peer_directory() -> PathBuf {
    std::env::temp_dir().join(format!("fireline-peers-{}.toml", Uuid::new_v4()))
}

#[tokio::test]
async fn mesh_baseline_exposes_peer_tools_and_prompts_remote_peer_over_acp() -> Result<()> {
    let peer_directory_path = temp_peer_directory();

    let handle_b = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "agent-b".to_string(),
        agent_command: vec![testy_bin()],
        state_stream: None,
        peer_directory_path: Some(peer_directory_path.clone()),
    })
    .await?;

    let handle_a = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "agent-a".to_string(),
        agent_command: vec![testy_bin()],
        state_stream: None,
        peer_directory_path: Some(peer_directory_path.clone()),
    })
    .await?;

    let tools = yopo::prompt(
        WebSocketTransport {
            url: handle_a.acp_url.clone(),
        },
        TestyCommand::ListTools {
            server: "fireline-peer".to_string(),
        }
        .to_prompt(),
    )
    .await?;

    assert!(
        tools.contains("list_peers") && tools.contains("prompt_peer"),
        "peer MCP server should expose list_peers and prompt_peer: {tools}"
    );

    let peers = yopo::prompt(
        WebSocketTransport {
            url: handle_a.acp_url.clone(),
        },
        TestyCommand::CallTool {
            server: "fireline-peer".to_string(),
            tool: "list_peers".to_string(),
            params: serde_json::json!({}),
        }
        .to_prompt(),
    )
    .await?;

    assert!(
        peers.contains("agent-a") && peers.contains("agent-b"),
        "list_peers should return both local runtimes: {peers}"
    );

    let prompt_peer = yopo::prompt(
        WebSocketTransport {
            url: handle_a.acp_url.clone(),
        },
        TestyCommand::CallTool {
            server: "fireline-peer".to_string(),
            tool: "prompt_peer".to_string(),
            params: serde_json::json!({
                "agentName": "agent-b",
                "prompt": TestyCommand::Echo {
                    message: "hello across mesh".to_string(),
                }
                .to_prompt(),
            }),
        }
        .to_prompt(),
    )
    .await?;

    assert!(
        prompt_peer.contains("agent-b") && prompt_peer.contains("hello across mesh"),
        "prompt_peer should return the remote peer response: {prompt_peer}"
    );

    let client = DsClient::new();
    let stream = client.stream(&handle_b.state_stream_url);
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
        body.contains("\"type\":\"prompt_turn\""),
        "remote peer runtime should record the cross-runtime prompt as a prompt_turn: {body}"
    );

    handle_a.shutdown().await?;
    handle_b.shutdown().await?;
    Ok(())
}
