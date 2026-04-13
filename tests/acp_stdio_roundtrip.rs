use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::{InitializeRequest, NewSessionRequest, PromptRequest, ProtocolVersion};
use agent_client_protocol_test::testy::TestyCommand;
use anyhow::{Context, Result, anyhow};
use durable_streams::{Client as DsClient, Offset};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use uuid::Uuid;

#[path = "support/stream_server.rs"]
mod stream_server;

#[tokio::test]
async fn acp_stdio_roundtrip_routes_prompt_and_state_over_native_stdio() -> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let state_stream = format!("fireline-stdio-roundtrip-{}", Uuid::new_v4());
    let prompt_text = format!("hello over stdio {}", now_ms());
    let mut child = spawn_fireline_stdio(&stream_server.base_url, &state_stream).await?;
    let stdin = child
        .stdin
        .take()
        .context("fireline stdio child stdin unavailable")?;
    let stdout = child
        .stdout
        .take()
        .context("fireline stdio child stdout unavailable")?;

    let mut stdin = stdin;
    let mut stdout = BufReader::new(stdout).lines();

    write_json_line(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": "init-1",
            "method": "initialize",
            "params": serde_json::to_value(InitializeRequest::new(ProtocolVersion::LATEST))?,
        }),
    )
    .await?;
    let initialize = read_response(&mut stdout, "init-1").await?;
    assert!(
        initialize.get("result").is_some(),
        "initialize should return a JSON-RPC result: {initialize}"
    );

    write_json_line(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": "session-1",
            "method": "session/new",
            "params": serde_json::to_value(NewSessionRequest::new(repo_root()))?,
        }),
    )
    .await?;
    let new_session = read_response(&mut stdout, "session-1").await?;
    let session_id = new_session
        .get("result")
        .and_then(|result| result.get("sessionId"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .context("session/new response missing result.sessionId")?;

    write_json_line(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": "prompt-1",
            "method": "session/prompt",
            "params": serde_json::to_value(PromptRequest::new(
                session_id.clone(),
                vec![TestyCommand::Echo {
                    message: prompt_text.clone(),
                }
                .to_prompt()
                .into()],
            ))?,
        }),
    )
    .await?;

    let prompt_response = read_prompt_roundtrip(&mut stdout, "prompt-1").await?;
    assert_eq!(
        prompt_response
            .response
            .get("result")
            .and_then(|result| result.get("stopReason"))
            .and_then(Value::as_str),
        Some("end_turn"),
        "session/prompt should resolve successfully over stdio: {:?}",
        prompt_response.response
    );
    assert!(
        prompt_response.agent_updates.iter().any(|notification| {
            notification
                .get("params")
                .and_then(|params| params.get("update"))
                .and_then(|update| update.get("content"))
                .and_then(|content| content.get("text"))
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains(&prompt_text))
        }),
        "session/update notifications should stream the echoed text over stdio: {:?}",
        prompt_response.agent_updates
    );

    let state_stream_url = stream_server.stream_url(&state_stream);
    let body = wait_for_stream_body(
        &state_stream_url,
        &[
            "\"type\":\"session_v2\"",
            "\"type\":\"prompt_request\"",
            "\"type\":\"chunk_v2\"",
            &prompt_text,
        ],
    )
    .await?;

    assert!(
        body.contains(&session_id),
        "stdio path should persist the created session into the durable state stream: {body}"
    );

    shutdown_child(&mut child).await;
    stream_server.shutdown().await;
    Ok(())
}

struct PromptRoundtrip {
    response: Value,
    agent_updates: Vec<Value>,
}

async fn spawn_fireline_stdio(durable_streams_url: &str, state_stream: &str) -> Result<Child> {
    let inherit_child_logs = std::env::var_os("FIRELINE_TEST_CHILD_LOGS").is_some();
    let mut command = Command::new(fireline_bin());
    command
        .arg("--acp-stdio")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg("0")
        .arg("--name")
        .arg("acp-stdio-roundtrip")
        .arg("--state-stream")
        .arg(state_stream)
        .arg("--durable-streams-url")
        .arg(durable_streams_url)
        .arg("--")
        .arg(testy_bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if inherit_child_logs {
        command.stderr(Stdio::inherit());
    } else {
        command.stderr(Stdio::null());
    }

    command
        .spawn()
        .context("spawn fireline --acp-stdio roundtrip child")
}

async fn write_json_line(stdin: &mut ChildStdin, value: &Value) -> Result<()> {
    let mut encoded = serde_json::to_vec(value)?;
    encoded.push(b'\n');
    stdin.write_all(&encoded).await?;
    stdin.flush().await?;
    Ok(())
}

async fn read_response(stdout: &mut Lines<BufReader<ChildStdout>>, id: &str) -> Result<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let message = read_json_line(stdout, deadline).await?;
        if message.get("id").and_then(Value::as_str) == Some(id) {
            return Ok(message);
        }
    }
}

async fn read_prompt_roundtrip(
    stdout: &mut Lines<BufReader<ChildStdout>>,
    prompt_id: &str,
) -> Result<PromptRoundtrip> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut agent_updates = Vec::new();

    loop {
        let message = read_json_line(stdout, deadline).await?;
        if message.get("id").and_then(Value::as_str) == Some(prompt_id) {
            return Ok(PromptRoundtrip {
                response: message,
                agent_updates,
            });
        }
        if message.get("method").and_then(Value::as_str) == Some("session/update") {
            agent_updates.push(message);
        }
    }
}

async fn read_json_line(
    stdout: &mut Lines<BufReader<ChildStdout>>,
    deadline: tokio::time::Instant,
) -> Result<Value> {
    loop {
        let next_line = tokio::time::timeout_at(deadline, stdout.next_line())
            .await
            .map_err(|_| anyhow!("timed out waiting for ACP stdio line"))??;
        let Some(line) = next_line else {
            return Err(anyhow!(
                "ACP stdio stream closed before the expected response"
            ));
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        return serde_json::from_str(trimmed).context("decode ACP stdio JSON line");
    }
}

async fn wait_for_stream_body(stream_url: &str, needles: &[&str]) -> Result<String> {
    let client = DsClient::new();
    let stream = client.stream(stream_url);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

    loop {
        let body = read_stream_body(&stream).await?;
        if needles.iter().all(|needle| body.contains(needle)) {
            return Ok(body);
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for stream body at {stream_url} to contain {needles:?}; body was {body}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn read_stream_body(stream: &durable_streams::DurableStream) -> Result<String> {
    let mut reader = stream.read().offset(Offset::Beginning).build()?;
    let mut body = String::new();
    while let Some(chunk) = reader.next_chunk().await? {
        body.push_str(std::str::from_utf8(&chunk.data)?);
        if chunk.up_to_date {
            break;
        }
    }
    Ok(body)
}

async fn shutdown_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
}

fn fireline_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fireline"))
}

fn testy_bin() -> String {
    PathBuf::from(env!("CARGO_BIN_EXE_fireline-testy"))
        .display()
        .to_string()
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as i64
}
