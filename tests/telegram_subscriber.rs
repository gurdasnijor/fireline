use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use durable_streams::{Client as DurableStreamsClient, CreateOptions, LiveMode, Offset, Producer};
use fireline_harness::{
    ActiveSubscriber, DurableSubscriber, HandlerOutcome, StreamEnvelope, TelegramParseMode,
    TelegramScope, TelegramSubscriber, TelegramSubscriberConfig,
    append_telegram_approval_resolution,
};
use serde_json::{Value, json};
use tokio::sync::{Mutex, oneshot};
use uuid::Uuid;

#[path = "support/stream_server.rs"]
mod stream_server;

#[derive(Debug, Clone)]
struct CapturedTelegramRequest {
    body: Value,
    headers: HeaderMap,
    method: String,
}

#[derive(Debug, Clone)]
struct TestTelegramState {
    next_message_id: Arc<Mutex<i64>>,
    requests: Arc<Mutex<Vec<CapturedTelegramRequest>>>,
    updates: Arc<Mutex<VecDeque<Vec<Value>>>>,
}

impl Default for TestTelegramState {
    fn default() -> Self {
        Self {
            next_message_id: Arc::new(Mutex::new(1_000)),
            requests: Arc::new(Mutex::new(Vec::new())),
            updates: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

struct TestTelegramServer {
    base_url: String,
    state: TestTelegramState,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl TestTelegramServer {
    async fn spawn() -> Result<Self> {
        async fn handler(
            Path((_bot, method)): Path<(String, String)>,
            State(state): State<TestTelegramState>,
            headers: HeaderMap,
            body: Bytes,
        ) -> (StatusCode, Json<Value>) {
            let payload = if body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null)
            };
            state.requests.lock().await.push(CapturedTelegramRequest {
                body: payload.clone(),
                headers,
                method: method.clone(),
            });

            let response = match method.as_str() {
                "sendMessage" => {
                    let mut next_message_id = state.next_message_id.lock().await;
                    *next_message_id += 1;
                    let message_id = *next_message_id;
                    let chat_id = payload
                        .get("chat_id")
                        .cloned()
                        .unwrap_or_else(|| Value::String("chat-1".to_string()));
                    json!({
                        "ok": true,
                        "result": {
                            "message_id": message_id,
                            "chat": { "id": chat_id }
                        }
                    })
                }
                "getUpdates" => {
                    let updates = state.updates.lock().await.pop_front().unwrap_or_default();
                    json!({ "ok": true, "result": updates })
                }
                "answerCallbackQuery" => json!({ "ok": true, "result": true }),
                "editMessageText" => json!({ "ok": true, "result": true }),
                other => json!({
                    "ok": false,
                    "description": format!("unsupported Telegram method {other}")
                }),
            };

            (StatusCode::OK, Json(response))
        }

        let state = TestTelegramState::default();
        let router = Router::new()
            .route("/{bot}/{method}", post(handler))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });

        Ok(Self {
            base_url: format!("http://127.0.0.1:{}", addr.port()),
            state,
            shutdown_tx: Some(shutdown_tx),
            task,
        })
    }

    async fn push_updates(&self, updates: Vec<Value>) {
        self.state.updates.lock().await.push_back(updates);
    }

    async fn wait_for_method(&self, method: &str) -> CapturedTelegramRequest {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(found) = self
                .state
                .requests
                .lock()
                .await
                .iter()
                .find(|request| request.method == method)
                .cloned()
            {
                return found;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("timed out waiting for Telegram method '{method}'");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn last_sent_message_id(&self) -> i64 {
        *self.state.next_message_id.lock().await
    }

    async fn captured_methods(&self) -> Vec<String> {
        self.state
            .requests
            .lock()
            .await
            .iter()
            .map(|request| request.method.clone())
            .collect()
    }

    async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = self.task.await;
    }
}

#[tokio::test]
async fn telegram_subscriber_posts_card_and_appends_approval_resolution() -> Result<()> {
    let telegram_server = TestTelegramServer::spawn().await?;
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let state_stream_url = stream_server.stream_url(&format!("telegram-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&state_stream_url).await?;

    let subscriber = TelegramSubscriber::new(TelegramSubscriberConfig {
        bot_token: "test-token".to_string(),
        api_base_url: telegram_server.base_url.clone(),
        chat_id: Some("chat-42".to_string()),
        allowed_user_ids: BTreeSet::from([String::from("42")]),
        approval_timeout: Some(Duration::from_secs(2)),
        poll_interval: Duration::from_millis(5),
        poll_timeout: Duration::ZERO,
        parse_mode: TelegramParseMode::Html,
        scope: TelegramScope::ToolCalls,
    });

    let event = permission_request_event()?;
    let request = subscriber
        .matches(&event)
        .context("Telegram subscriber should match permission_request envelopes")?;
    let subscriber_task = {
        let subscriber = subscriber.clone();
        tokio::spawn(async move { subscriber.handle(request).await })
    };

    let send_message = telegram_server.wait_for_method("sendMessage").await;
    assert_eq!(
        send_message
            .headers
            .get("traceparent")
            .and_then(|value| value.to_str().ok()),
        Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"),
        "INVARIANT (TelegramSubscriber): outbound Telegram side effects must propagate traceparent"
    );
    assert_eq!(
        send_message.body.get("chat_id").and_then(Value::as_str),
        Some("chat-42")
    );
    let send_text = send_message
        .body
        .get("text")
        .and_then(Value::as_str)
        .context("sendMessage text")?;
    assert!(
        send_text.contains("Fireline approval required")
            && send_text.contains("session-1")
            && send_text.contains("req-1"),
        "approval card should contain the canonical session/request ids and the approval banner"
    );
    assert_eq!(
        send_message
            .body
            .pointer("/reply_markup/inline_keyboard/0/0/text"),
        Some(&Value::String("Approve".to_string()))
    );
    assert_eq!(
        send_message
            .body
            .pointer("/reply_markup/inline_keyboard/0/1/text"),
        Some(&Value::String("Deny".to_string()))
    );

    let message_id = telegram_server.last_sent_message_id().await;
    telegram_server
        .push_updates(vec![json!({
            "update_id": 1,
            "callback_query": {
                "id": "callback-1",
                "data": "approve",
                "from": {
                    "id": 42,
                    "username": "operator"
                },
                "message": {
                    "message_id": message_id,
                    "chat": { "id": "chat-42" }
                }
            }
        })])
        .await;

    let completion = match subscriber_task
        .await
        .map_err(|error| anyhow!("Telegram subscriber task panicked: {error}"))?
    {
        HandlerOutcome::Completed(completion) => completion,
        outcome => panic!("expected Telegram approval resolution, got {outcome:?}"),
    };

    assert_eq!(completion.kind, "approval_resolved");
    assert_eq!(completion.session_id.to_string(), "session-1");
    assert_eq!(
        serde_json::to_value(&completion.request_id)?,
        Value::String("req-1".to_string())
    );
    assert!(completion.allow);
    assert_eq!(completion.resolved_by, "telegram:@operator");
    assert_eq!(
        completion
            .meta
            .as_ref()
            .and_then(|meta| meta.traceparent.as_deref()),
        Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01")
    );

    let producer = json_producer(&state_stream_url, "telegram-subscriber-test");
    append_telegram_approval_resolution(&producer, &completion).await?;
    let rows = read_all_rows(&state_stream_url).await?;
    assert_eq!(rows.len(), 1);
    let stored = rows.into_iter().next().expect("stored Telegram resolution");
    assert_eq!(
        stored.get("type").and_then(Value::as_str),
        Some("permission")
    );
    assert_eq!(
        stored
            .get("value")
            .and_then(|value| value.get("kind"))
            .and_then(Value::as_str),
        Some("approval_resolved")
    );
    assert_eq!(
        stored
            .get("value")
            .and_then(|value| value.get("resolvedBy"))
            .and_then(Value::as_str),
        Some("telegram:@operator")
    );
    assert_eq!(
        stored
            .get("value")
            .and_then(|value| value.get("_meta"))
            .and_then(|value| value.get("traceparent"))
            .and_then(Value::as_str),
        Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"),
        "INVARIANT (TelegramSubscriber): approval_resolved completion must preserve source trace context"
    );

    let methods = telegram_server.captured_methods().await;
    assert!(
        methods.iter().any(|method| method == "answerCallbackQuery")
            && methods.iter().any(|method| method == "editMessageText"),
        "approval tap should acknowledge the callback and edit the inline-card message"
    );

    telegram_server.shutdown().await;
    stream_server.shutdown().await;
    Ok(())
}

fn permission_request_event() -> Result<StreamEnvelope> {
    Ok(StreamEnvelope::from_json(json!({
        "type": "permission",
        "key": "session-1:req-1",
        "headers": { "operation": "insert" },
        "value": {
            "kind": "permission_request",
            "sessionId": "session-1",
            "requestId": "req-1",
            "reason": "approval required for rm -rf dist",
            "createdAtMs": 1,
            "_meta": {
                "traceparent": "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01",
                "tracestate": "vendor=value",
                "baggage": "tenant=acme"
            }
        }
    }))
    .context("decode permission_request event")?)
}

async fn ensure_json_stream_exists(stream_url: &str) -> Result<()> {
    let client = DurableStreamsClient::new();
    let stream = client.stream(stream_url);
    stream
        .create_with(CreateOptions::new().content_type("application/json"))
        .await
        .map(|_| ())
        .or_else(|error| match error {
            durable_streams::StreamError::Conflict => Ok(()),
            other => Err(other),
        })
        .with_context(|| format!("create durable stream '{stream_url}'"))
}

fn json_producer(stream_url: &str, producer_name: &str) -> Producer {
    let client = DurableStreamsClient::new();
    let mut stream = client.stream(stream_url);
    stream.set_content_type("application/json");
    stream
        .producer(format!("{producer_name}-{}", Uuid::new_v4()))
        .content_type("application/json")
        .build()
}

async fn read_all_rows(stream_url: &str) -> Result<Vec<Value>> {
    let client = DurableStreamsClient::new();
    let stream = client.stream(stream_url);
    let mut reader = stream
        .read()
        .offset(Offset::Beginning)
        .live(LiveMode::Off)
        .build()
        .with_context(|| format!("build durable stream reader for '{stream_url}'"))?;

    let mut rows = Vec::new();
    loop {
        let Some(chunk) = reader
            .next_chunk()
            .await
            .with_context(|| format!("read durable stream '{stream_url}'"))?
        else {
            break;
        };
        if !chunk.data.is_empty() {
            rows.extend(
                serde_json::from_slice::<Vec<Value>>(&chunk.data)
                    .context("decode durable stream rows as JSON")?,
            );
        }
        if chunk.up_to_date {
            break;
        }
    }
    Ok(rows)
}
