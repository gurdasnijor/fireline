use std::collections::{BTreeSet, HashMap, VecDeque};
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
use durable_streams::{Client as DurableStreamsClient, LiveMode, Offset};
use fireline_harness::telegram_subscriber::{TelegramCursorRecord, TelegramDeadLetterRecord};
use fireline_harness::{
    ActiveSubscriber, DurableSubscriber, HandlerOutcome, RetryPolicy, StreamEnvelope,
    TelegramParseMode, TelegramScope, TelegramSubscriber, TelegramSubscriberConfig,
};
use serde::{Deserialize, Serialize};
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
struct StubTelegramResponse {
    status: StatusCode,
    body: Value,
}

#[derive(Debug, Clone)]
struct TestTelegramState {
    next_message_id: Arc<Mutex<i64>>,
    requests: Arc<Mutex<Vec<CapturedTelegramRequest>>>,
    updates: Arc<Mutex<VecDeque<Vec<Value>>>>,
    responses: Arc<Mutex<HashMap<String, VecDeque<StubTelegramResponse>>>>,
}

impl Default for TestTelegramState {
    fn default() -> Self {
        Self {
            next_message_id: Arc::new(Mutex::new(1_000)),
            requests: Arc::new(Mutex::new(Vec::new())),
            updates: Arc::new(Mutex::new(VecDeque::new())),
            responses: Arc::new(Mutex::new(HashMap::new())),
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

            if let Some(response) = state
                .responses
                .lock()
                .await
                .get_mut(&method)
                .and_then(VecDeque::pop_front)
            {
                return (response.status, Json(response.body));
            }

            let response = match method.as_str() {
                "sendMessage" => {
                    let mut next_message_id = state.next_message_id.lock().await;
                    *next_message_id += 1;
                    let message_id = *next_message_id;
                    let chat_id = payload
                        .get("chat_id")
                        .cloned()
                        .unwrap_or_else(|| Value::String("chat-1".to_string()));
                    (
                        StatusCode::OK,
                        json!({
                            "ok": true,
                            "result": {
                                "message_id": message_id,
                                "chat": { "id": chat_id }
                            }
                        }),
                    )
                }
                "getUpdates" => {
                    let updates = state.updates.lock().await.pop_front().unwrap_or_default();
                    (StatusCode::OK, json!({ "ok": true, "result": updates }))
                }
                "answerCallbackQuery" => (StatusCode::OK, json!({ "ok": true, "result": true })),
                "editMessageText" => (StatusCode::OK, json!({ "ok": true, "result": true })),
                other => (
                    StatusCode::OK,
                    json!({
                        "ok": false,
                        "description": format!("unsupported Telegram method {other}")
                    }),
                ),
            };

            (response.0, Json(response.1))
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

    async fn queue_response(&self, method: &str, status: StatusCode, body: Value) {
        self.state
            .responses
            .lock()
            .await
            .entry(method.to_string())
            .or_default()
            .push_back(StubTelegramResponse { status, body });
    }

    async fn wait_for_method_count(
        &self,
        method: &str,
        count: usize,
    ) -> Vec<CapturedTelegramRequest> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let requests: Vec<_> = self
                .state
                .requests
                .lock()
                .await
                .iter()
                .filter(|request| request.method == method)
                .cloned()
                .collect();
            if requests.len() >= count {
                return requests;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("timed out waiting for {count} Telegram '{method}' call(s)");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn method_count(&self, method: &str) -> usize {
        self.state
            .requests
            .lock()
            .await
            .iter()
            .filter(|request| request.method == method)
            .count()
    }

    async fn last_sent_message_id(&self) -> i64 {
        *self.state.next_message_id.lock().await
    }

    async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = self.task.await;
    }
}

#[tokio::test]
async fn telegram_retry_resumes_inflight_without_duplicate_send() -> Result<()> {
    let telegram_server = TestTelegramServer::spawn().await?;
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let cursor_stream_url =
        stream_server.stream_url(&format!("telegram-cursor-{}", Uuid::new_v4()));
    let dead_letter_stream_url =
        stream_server.stream_url(&format!("telegram-dead-letter-{}", Uuid::new_v4()));
    let subscriber = hardened_subscriber(
        &telegram_server,
        Some(cursor_stream_url.clone()),
        Some(dead_letter_stream_url),
    );
    telegram_server
        .queue_response(
            "getUpdates",
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "ok": false, "description": "transient polling outage" }),
        )
        .await;

    let request = subscriber
        .matches(&permission_request_event(
            "0000000000000001_0000000000000000",
        )?)
        .context("DSV-01 CompletionKeyUnique: Telegram subscriber should match permission_request envelopes before durable retry handling")?;
    let subscriber_task = {
        let subscriber = subscriber.clone();
        tokio::spawn(async move { subscriber.handle(request).await })
    };

    let send_message = telegram_server
        .wait_for_method_count("sendMessage", 1)
        .await;
    assert_eq!(send_message.len(), 1);
    assert_eq!(
        send_message[0]
            .headers
            .get("traceparent")
            .and_then(|value| value.to_str().ok()),
        Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"),
        "DSV-05 TraceContextPropagated: retryable Telegram sendMessage must carry the source traceparent",
    );

    telegram_server.wait_for_method_count("getUpdates", 1).await;
    let message_id = telegram_server.last_sent_message_id().await;
    telegram_server
        .push_updates(vec![callback_update(1, message_id, "chat-42", "approve")])
        .await;

    let completion = match subscriber_task
        .await
        .map_err(|error| anyhow!("Telegram hardening task panicked: {error}"))?
    {
        HandlerOutcome::Completed(completion) => completion,
        outcome => panic!(
            "DSV-03 RetryBounded: expected retrying Telegram dispatch to complete, got {outcome:?}"
        ),
    };

    assert_eq!(
        telegram_server.method_count("sendMessage").await,
        1,
        "DSV-01 CompletionKeyUnique: transient polling retry must resume the in-flight approval card instead of posting a duplicate sendMessage",
    );
    assert_eq!(
        completion
            .meta
            .as_ref()
            .and_then(|meta| meta.traceparent.as_deref()),
        Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"),
        "DSV-05 TraceContextPropagated: retried Telegram completion must preserve traceparent",
    );

    let cursor = latest_cursor(&cursor_stream_url)
        .await?
        .context("cursor record")?;
    assert_eq!(cursor.next_update_id, Some(2));
    assert!(cursor.in_flight.is_none());

    telegram_server.shutdown().await;
    stream_server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn telegram_persistent_failures_dead_letter_and_gate_replay() -> Result<()> {
    let telegram_server = TestTelegramServer::spawn().await?;
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let cursor_stream_url =
        stream_server.stream_url(&format!("telegram-cursor-{}", Uuid::new_v4()));
    let dead_letter_stream_url =
        stream_server.stream_url(&format!("telegram-dead-letter-{}", Uuid::new_v4()));
    let subscriber = hardened_subscriber(
        &telegram_server,
        Some(cursor_stream_url.clone()),
        Some(dead_letter_stream_url.clone()),
    );

    for _ in 0..3 {
        telegram_server
            .queue_response(
                "getUpdates",
                StatusCode::SERVICE_UNAVAILABLE,
                json!({ "ok": false, "description": "persistent polling outage" }),
            )
            .await;
    }

    let request = subscriber
        .matches(&permission_request_event(
            "0000000000000002_0000000000000000",
        )?)
        .context("match Telegram permission_request")?;
    let outcome = subscriber.handle(request.clone()).await;
    let error = match outcome {
        HandlerOutcome::Failed(error) => error,
        other => panic!(
            "DSV-04 DeadLetterTerminal: expected Telegram retry exhaustion to fail terminally, got {other:?}"
        ),
    };
    assert!(
        error.to_string().contains("dead-lettered"),
        "DSV-04 DeadLetterTerminal: terminal Telegram retry exhaustion must surface a dead-letter outcome",
    );
    assert_eq!(
        telegram_server.method_count("sendMessage").await,
        1,
        "DSV-01 CompletionKeyUnique: retry exhaustion after a successful sendMessage must not post duplicate approval cards",
    );
    assert_eq!(telegram_server.method_count("getUpdates").await, 3);

    let record = latest_dead_letter(&dead_letter_stream_url)
        .await?
        .context("dead-letter record")?;
    assert_eq!(record.attempts, 3);
    assert_eq!(record.completion_key, request.completion_key());
    assert_eq!(
        record
            .meta
            .as_ref()
            .and_then(|meta| meta.traceparent.as_deref()),
        Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"),
        "DSV-05 TraceContextPropagated: Telegram dead-letter rows must preserve trace lineage",
    );

    let replay_outcome = subscriber.handle(request).await;
    match replay_outcome {
        HandlerOutcome::Failed(error) => assert!(
            error.to_string().contains("already dead-lettered"),
            "DSV-04 DeadLetterTerminal: replay after durable dead-letter must short-circuit before any new delivery attempt"
        ),
        other => panic!("expected replayed Telegram dead-letter to fail fast, got {other:?}"),
    }
    assert_eq!(
        telegram_server.method_count("sendMessage").await,
        1,
        "DSV-04 DeadLetterTerminal: replay after dead-letter must not send a new Telegram card",
    );
    assert_eq!(
        telegram_server.method_count("getUpdates").await,
        3,
        "DSV-04 DeadLetterTerminal: replay after dead-letter must not poll Telegram again",
    );

    let cursor = latest_cursor(&cursor_stream_url)
        .await?
        .context("cursor record")?;
    assert!(cursor.in_flight.is_none());

    telegram_server.shutdown().await;
    stream_server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn telegram_restart_replays_from_persisted_offset_without_duplicate_send() -> Result<()> {
    let telegram_server = TestTelegramServer::spawn().await?;
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let cursor_stream_url =
        stream_server.stream_url(&format!("telegram-cursor-{}", Uuid::new_v4()));
    let dead_letter_stream_url =
        stream_server.stream_url(&format!("telegram-dead-letter-{}", Uuid::new_v4()));
    let first_subscriber = hardened_subscriber(
        &telegram_server,
        Some(cursor_stream_url.clone()),
        Some(dead_letter_stream_url),
    );
    let request = first_subscriber
        .matches(&permission_request_event(
            "0000000000000003_0000000000000000",
        )?)
        .context("match Telegram permission_request")?;

    let first_task = {
        let subscriber = first_subscriber.clone();
        let request = request.clone();
        tokio::spawn(async move { subscriber.handle(request).await })
    };

    telegram_server
        .wait_for_method_count("sendMessage", 1)
        .await;
    telegram_server
        .push_updates(vec![json!({
            "update_id": 7,
            "message": {
                "message_id": 700,
                "chat": { "id": "chat-42" }
            }
        })])
        .await;
    wait_for_cursor_offset(&cursor_stream_url, 8).await?;

    first_task.abort();

    let second_subscriber =
        hardened_subscriber(&telegram_server, Some(cursor_stream_url.clone()), None);
    let message_id = telegram_server.last_sent_message_id().await;
    telegram_server
        .push_updates(vec![callback_update(8, message_id, "chat-42", "approve")])
        .await;

    let resumed_task = {
        let subscriber = second_subscriber.clone();
        tokio::spawn(async move { subscriber.handle(request).await })
    };

    let completion = match resumed_task
        .await
        .map_err(|error| anyhow!("resumed Telegram task panicked: {error}"))?
    {
        HandlerOutcome::Completed(completion) => completion,
        outcome => panic!(
            "DSV-02 ReplayIdempotent: expected restarted Telegram subscriber to converge, got {outcome:?}"
        ),
    };

    let get_updates = telegram_server.wait_for_method_count("getUpdates", 2).await;
    assert!(
        get_updates
            .iter()
            .any(|request| request.body.get("offset") == Some(&Value::from(8))),
        "DSV-02 ReplayIdempotent: restarted Telegram polling must resume from the durable offset instead of replaying older updates",
    );
    assert_eq!(
        telegram_server.method_count("sendMessage").await,
        1,
        "DSV-01 CompletionKeyUnique: restart replay must reuse the persisted in-flight Telegram card instead of posting a duplicate sendMessage",
    );
    assert_eq!(completion.allow, true);

    telegram_server.shutdown().await;
    stream_server.shutdown().await;
    Ok(())
}

fn hardened_subscriber(
    telegram_server: &TestTelegramServer,
    cursor_stream: Option<String>,
    dead_letter_stream: Option<String>,
) -> TelegramSubscriber {
    TelegramSubscriber::new(TelegramSubscriberConfig {
        bot_token: "test-token".to_string(),
        api_base_url: telegram_server.base_url.clone(),
        chat_id: Some("chat-42".to_string()),
        allowed_user_ids: BTreeSet::from([String::from("42")]),
        approval_timeout: Some(Duration::from_secs(2)),
        poll_interval: Duration::from_millis(5),
        poll_timeout: Duration::ZERO,
        parse_mode: TelegramParseMode::Html,
        scope: TelegramScope::ToolCalls,
        cursor_stream,
        dead_letter_stream,
        retry_policy: Some(RetryPolicy {
            max_attempts: 3,
            initial_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
        }),
    })
}

fn callback_update(update_id: i64, message_id: i64, chat_id: &str, data: &str) -> Value {
    json!({
        "update_id": update_id,
        "callback_query": {
            "id": format!("callback-{update_id}"),
            "data": data,
            "from": {
                "id": 42,
                "username": "operator"
            },
            "message": {
                "message_id": message_id,
                "chat": { "id": chat_id }
            }
        }
    })
}

fn permission_request_event(offset: &str) -> Result<StreamEnvelope> {
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
    .context("decode permission_request event")?
    .with_source_offset(Offset::at(offset)))
}

async fn wait_for_cursor_offset(stream_url: &str, next_update_id: i64) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if latest_cursor(stream_url)
            .await?
            .is_some_and(|cursor| cursor.next_update_id == Some(next_update_id))
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for Telegram cursor offset {next_update_id}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn latest_cursor(stream_url: &str) -> Result<Option<TelegramCursorRecord>> {
    let rows = read_all_rows(stream_url).await?;
    let mut latest = None;
    for row in rows {
        let envelope: StateEnvelope<TelegramCursorRecord> =
            serde_json::from_value(row).context("decode Telegram cursor row")?;
        if envelope.entity_type == "telegram_cursor" {
            latest = Some(envelope.value);
        }
    }
    Ok(latest)
}

async fn latest_dead_letter(stream_url: &str) -> Result<Option<TelegramDeadLetterRecord>> {
    let rows = read_all_rows(stream_url).await?;
    let mut latest = None;
    for row in rows {
        let envelope: StateEnvelope<TelegramDeadLetterRecord> =
            serde_json::from_value(row).context("decode Telegram dead-letter row")?;
        if envelope.entity_type == "telegram_dead_letter" {
            latest = Some(envelope.value);
        }
    }
    Ok(latest)
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StateEnvelope<T> {
    #[serde(rename = "type")]
    entity_type: String,
    key: String,
    headers: StateHeaders,
    value: T,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StateHeaders {
    operation: String,
}
