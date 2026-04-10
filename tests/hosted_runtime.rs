use std::net::IpAddr;
use std::time::Duration;

use anyhow::Result;
use durable_streams::{Client as DsClient, Offset};
use fireline::bootstrap::{BootstrapConfig, start};

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

#[tokio::test]
async fn hosted_runtime_serves_acp_and_emits_state_events() -> Result<()> {
    let handle = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "hosted-test".to_string(),
        runtime_key: None,
        node_id: None,
        agent_command: vec![testy_bin()],
        state_stream: None,
        peer_directory_path: None,
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
