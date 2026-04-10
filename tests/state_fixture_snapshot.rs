use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use durable_streams::{Client as DsClient, Offset};
use fireline::bootstrap::{BootstrapConfig, start};
use fireline_conductor::topology::TopologySpec;
use serde_json::Value;
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
    PathBuf::from(env!("CARGO_BIN_EXE_fireline-testy"))
        .display()
        .to_string()
}

fn temp_peer_directory() -> PathBuf {
    std::env::temp_dir().join(format!("fireline-peers-{}.toml", Uuid::new_v4()))
}

#[tokio::test]
#[ignore = "updates the committed NDJSON conformance fixture for @fireline/state"]
async fn update_rust_state_fixture_snapshot() -> Result<()> {
    let peer_directory_path = temp_peer_directory();

    let handle_b = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "fixture-child".to_string(),
        runtime_key: format!("runtime:{}", Uuid::new_v4()),
        node_id: "node:test-fixture".to_string(),
        agent_command: vec![testy_bin()],
        state_stream: Some(format!("fireline-fixture-child-{}", Uuid::new_v4())),
        external_stream_base_url: None,
        advertised_acp_url: None,
        stream_storage: None,
        peer_directory_path: peer_directory_path.clone(),
        topology: TopologySpec::default(),
    })
    .await?;

    let handle_a = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "fixture-snapshot".to_string(),
        runtime_key: format!("runtime:{}", Uuid::new_v4()),
        node_id: "node:test-fixture".to_string(),
        agent_command: vec![testy_bin()],
        state_stream: Some(format!("fireline-fixture-{}", Uuid::new_v4())),
        external_stream_base_url: None,
        advertised_acp_url: None,
        stream_storage: None,
        peer_directory_path,
        topology: TopologySpec::default(),
    })
    .await?;

    let response = yopo::prompt(
        WebSocketTransport {
            url: handle_a.acp_url.clone(),
        },
        agent_client_protocol_test::testy::TestyCommand::CallTool {
            server: "fireline-peer".to_string(),
            tool: "prompt_peer".to_string(),
            params: serde_json::json!({
                "agentName": "fixture-child",
                "prompt": agent_client_protocol_test::testy::TestyCommand::Echo {
                    message: "fixture snapshot prompt".to_string(),
                }
                .to_prompt(),
            }),
        }
        .to_prompt(),
    )
    .await?;
    assert!(response.contains("fixture-child"));
    assert!(response.contains("fixture snapshot prompt"));

    let client = DsClient::new();
    let stream = client.stream(&handle_a.state_stream_url);
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

        if body.contains("\"type\":\"prompt_turn\"")
            && body.contains("\"type\":\"session\"")
            && body.contains("\"type\":\"chunk\"")
            && body.contains("\"type\":\"child_session_edge\"")
        {
            break;
        }

        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for fixture-worthy state stream output");
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("packages")
        .join("state")
        .join("test")
        .join("fixtures")
        .join("rust-state-producer.ndjson");
    if let Some(parent) = fixture_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let ndjson = match serde_json::from_str::<Vec<Value>>(&body) {
        Ok(events) => events
            .into_iter()
            .map(|event| serde_json::to_string(&event))
            .collect::<Result<Vec<_>, _>>()?
            .join("\n"),
        Err(_) => body
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
    };

    std::fs::write(&fixture_path, format!("{ndjson}\n"))?;

    handle_a.shutdown().await?;
    handle_b.shutdown().await?;
    Ok(())
}
