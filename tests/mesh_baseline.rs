use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use agent_client_protocol_test::testy::TestyCommand;
use anyhow::Result;
use durable_streams::{Client as DsClient, Offset};
use fireline_harness::TopologySpec;
use fireline_host::bootstrap::{BootstrapConfig, start};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use uuid::Uuid;

#[path = "support/stream_server.rs"]
mod stream_server;

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
    let stream_server = stream_server::TestStreamServer::spawn().await?;

    let handle_b = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "agent-b".to_string(),
        runtime_key: format!("runtime:{}", Uuid::new_v4()),
        node_id: "node:test-mesh".to_string(),
        agent_command: vec![testy_bin()],
        mounted_resources: Vec::new(),
        state_stream: None,
        durable_streams_url: stream_server.base_url.clone(),
        peer_directory_path: peer_directory_path.clone(),
        control_plane_url: None,
        topology: TopologySpec::default(),
    })
    .await?;

    let handle_a = start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: "agent-a".to_string(),
        runtime_key: format!("runtime:{}", Uuid::new_v4()),
        node_id: "node:test-mesh".to_string(),
        agent_command: vec![testy_bin()],
        mounted_resources: Vec::new(),
        state_stream: None,
        durable_streams_url: stream_server.base_url.clone(),
        peer_directory_path: peer_directory_path.clone(),
        control_plane_url: None,
        topology: TopologySpec::default(),
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
    let stream_a = client.stream(&handle_a.state_stream_url);
    let stream_b = client.stream(&handle_b.state_stream_url);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut body_a = String::new();
    let mut body_b = String::new();

    loop {
        body_a.clear();
        body_b.clear();

        let mut reader_a = stream_a.read().offset(Offset::Beginning).build()?;
        while let Some(chunk) = reader_a.next_chunk().await? {
            body_a.push_str(std::str::from_utf8(&chunk.data)?);
            if chunk.up_to_date {
                break;
            }
        }

        let mut reader_b = stream_b.read().offset(Offset::Beginning).build()?;
        while let Some(chunk) = reader_b.next_chunk().await? {
            body_b.push_str(std::str::from_utf8(&chunk.data)?);
            if chunk.up_to_date {
                break;
            }
        }

        let parent = find_prompt_turn(&body_a, |text| text.contains("\"tool\":\"prompt_peer\""));
        let child = find_prompt_turn(&body_b, |text| text.contains("hello across mesh"));
        let edge = find_child_session_edge(&body_a);

        if let (Some(parent), Some(child), Some(edge)) = (parent, child, edge) {
            assert_eq!(
                child.parent_prompt_turn_id.as_deref(),
                Some(parent.prompt_turn_id.as_str()),
                "child prompt turn should point at the parent prompt turn: {body_b}"
            );
            assert_eq!(
                child.trace_id.as_deref(),
                parent.trace_id.as_deref(),
                "child prompt turn should inherit the parent trace id: {body_b}"
            );
            assert_eq!(
                edge.parent_runtime_id, handle_a.runtime_id,
                "child_session_edge should point at the parent runtime: {body_a}"
            );
            assert_eq!(
                edge.parent_session_id, parent.session_id,
                "child_session_edge should point at the parent session: {body_a}"
            );
            assert_eq!(
                edge.parent_prompt_turn_id, parent.prompt_turn_id,
                "child_session_edge should point at the parent turn: {body_a}"
            );
            assert_eq!(
                edge.child_runtime_id, handle_b.runtime_id,
                "child_session_edge should point at the child runtime: {body_a}"
            );
            assert_eq!(
                edge.child_session_id, child.session_id,
                "child_session_edge should point at the remote child session: {body_a}"
            );
            assert_eq!(
                edge.trace_id.as_deref(),
                parent.trace_id.as_deref(),
                "child_session_edge should carry the parent trace id: {body_a}"
            );
            break;
        }

        if tokio::time::Instant::now() >= deadline {
            break;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert!(
        body_b.contains("\"type\":\"prompt_turn\""),
        "remote peer runtime should record the cross-runtime prompt as a prompt_turn: {body_b}"
    );
    assert!(
        find_prompt_turn(&body_a, |text| text.contains("\"tool\":\"prompt_peer\"")).is_some(),
        "missing parent prompt turn in runtime A: {body_a}"
    );
    assert!(
        find_prompt_turn(&body_b, |text| text.contains("hello across mesh")).is_some(),
        "missing child prompt turn in runtime B: {body_b}"
    );
    assert!(
        find_child_session_edge(&body_a).is_some(),
        "missing child_session_edge in runtime A: {body_a}"
    );

    handle_a.shutdown().await?;
    handle_b.shutdown().await?;
    stream_server.shutdown().await;
    Ok(())
}

#[derive(Debug)]
struct PromptTurnEvent {
    prompt_turn_id: String,
    session_id: String,
    trace_id: Option<String>,
    parent_prompt_turn_id: Option<String>,
}

#[derive(Debug)]
struct ChildSessionEdgeEvent {
    parent_runtime_id: String,
    parent_session_id: String,
    parent_prompt_turn_id: String,
    child_runtime_id: String,
    child_session_id: String,
    trace_id: Option<String>,
}

fn find_prompt_turn(body: &str, predicate: impl Fn(&str) -> bool) -> Option<PromptTurnEvent> {
    parse_state_events(body).into_iter().find_map(|event| {
        if event.get("type")?.as_str()? != "prompt_turn" {
            return None;
        }

        let value = event.get("value")?;
        let text = value.get("text").and_then(Value::as_str).unwrap_or("");
        if !predicate(text) {
            return None;
        }

        Some(PromptTurnEvent {
            prompt_turn_id: value.get("promptTurnId")?.as_str()?.to_string(),
            session_id: value.get("sessionId")?.as_str()?.to_string(),
            trace_id: value
                .get("traceId")
                .and_then(Value::as_str)
                .map(str::to_string),
            parent_prompt_turn_id: value
                .get("parentPromptTurnId")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    })
}

fn find_child_session_edge(body: &str) -> Option<ChildSessionEdgeEvent> {
    parse_state_events(body).into_iter().find_map(|event| {
        if event.get("type")?.as_str()? != "child_session_edge" {
            return None;
        }

        let value = event.get("value")?;
        Some(ChildSessionEdgeEvent {
            parent_runtime_id: value.get("parentRuntimeId")?.as_str()?.to_string(),
            parent_session_id: value.get("parentSessionId")?.as_str()?.to_string(),
            parent_prompt_turn_id: value.get("parentPromptTurnId")?.as_str()?.to_string(),
            child_runtime_id: value.get("childRuntimeId")?.as_str()?.to_string(),
            child_session_id: value.get("childSessionId")?.as_str()?.to_string(),
            trace_id: value
                .get("traceId")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    })
}

fn parse_state_events(body: &str) -> Vec<Value> {
    match serde_json::from_str::<Value>(body) {
        Ok(Value::Array(events)) => events,
        Ok(value) => vec![value],
        Err(_) => {
            let mut stream = serde_json::Deserializer::from_str(body).into_iter::<Value>();
            std::iter::from_fn(move || stream.next())
                .filter_map(|result| result.ok())
                .collect()
        }
    }
}
