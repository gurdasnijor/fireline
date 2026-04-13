#[path = "support/stream_server.rs"]
mod stream_server;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use durable_streams::{Client as DurableStreamsClient, CreateOptions, LiveMode, Offset, Producer};
use fireline_harness::{
    ActiveSubscriber, AlwaysOnDeploymentSubscriber, AutoApproveConfig, AutoApproveSubscriber,
    CompletionKey, DeploymentWakeHandler, DeploymentWakeRequested, DurableSubscriber,
    HandlerOutcome, PEER_DELIVERY_ACK_ENTITY_TYPE, PeerDispatchSuccess, PeerRoutingDispatcher,
    PeerRoutingEvent, PeerRoutingSubscriber, ProvisionedRuntime, StreamEnvelope,
    TelegramParseMode, TelegramScope, TelegramSubscriber, TelegramSubscriberConfig, TraceContext,
    WebhookEventSelector, WebhookSubscriber, WebhookSubscriberConfig, WebhookTargetConfig,
    append_telegram_approval_resolution, append_webhook_completion,
};
use sacp::schema::{RequestId, SessionId, SessionUpdate, ToolCall, ToolCallId};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::sync::{Mutex, oneshot};
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
struct RenderedTopology {
    components: Vec<RenderedComponent>,
}

#[derive(Debug, Clone, Deserialize)]
struct RenderedComponent {
    name: String,
    #[serde(default)]
    config: Option<Value>,
}

impl RenderedTopology {
    fn component(&self, name: &str) -> Result<&RenderedComponent> {
        self.components
            .iter()
            .find(|component| component.name == name)
            .with_context(|| format!("missing topology component '{name}'"))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebhookComponentConfig {
    target: String,
    events: Vec<WebhookEventSelectorWire>,
    target_config: WebhookTargetConfigWire,
    #[serde(default)]
    retry_policy: Option<RetryPolicyWire>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebhookTargetConfigWire {
    url: String,
    headers: BTreeMap<String, String>,
    timeout_ms: u64,
    max_attempts: u32,
    cursor_stream: String,
    #[serde(default)]
    dead_letter_stream: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RetryPolicyWire {
    #[serde(default)]
    max_attempts: Option<u32>,
    #[serde(default)]
    initial_backoff_ms: Option<u64>,
    #[serde(default)]
    max_backoff_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TelegramComponentConfig {
    #[serde(default)]
    bot_token: Option<String>,
    #[serde(default)]
    chat_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum WebhookEventSelectorWire {
    Kind {
        kind: String,
    },
    Exact {
        exact: WebhookExactSelectorWire,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebhookExactSelectorWire {
    entity_type: String,
    #[serde(default)]
    kind: Option<String>,
}

#[derive(Debug, Clone)]
struct CapturedWebhookRequest {
    headers: HeaderMap,
    payload: Value,
}

#[derive(Debug, Clone, Default)]
struct WebhookServerState {
    requests: Arc<Mutex<Vec<CapturedWebhookRequest>>>,
}

struct TestWebhookServer {
    url: String,
    state: WebhookServerState,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl TestWebhookServer {
    async fn spawn() -> Result<Self> {
        async fn handler(
            State(state): State<WebhookServerState>,
            headers: HeaderMap,
            body: Bytes,
        ) -> (StatusCode, Json<Value>) {
            let payload = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            state
                .requests
                .lock()
                .await
                .push(CapturedWebhookRequest { headers, payload });
            (StatusCode::OK, Json(json!({ "ok": true })))
        }

        let state = WebhookServerState::default();
        let router = Router::new().route("/hooks/durable-subscriber", post(handler)).with_state(state.clone());
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
            url: format!("http://127.0.0.1:{}/hooks/durable-subscriber", addr.port()),
            state,
            shutdown_tx: Some(shutdown_tx),
            task,
        })
    }

    async fn captured(&self) -> Vec<CapturedWebhookRequest> {
        self.state.requests.lock().await.clone()
    }

    async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = self.task.await;
    }
}

#[derive(Debug, Clone)]
struct CapturedTelegramRequest {
    body: Value,
    headers: HeaderMap,
    method: String,
}

#[derive(Debug, Clone)]
struct TelegramServerState {
    next_message_id: Arc<Mutex<i64>>,
    requests: Arc<Mutex<Vec<CapturedTelegramRequest>>>,
    updates: Arc<Mutex<VecDeque<Vec<Value>>>>,
}

impl Default for TelegramServerState {
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
    state: TelegramServerState,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl TestTelegramServer {
    async fn spawn() -> Result<Self> {
        async fn handler(
            Path((_bot, method)): Path<(String, String)>,
            State(state): State<TelegramServerState>,
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
                        .unwrap_or_else(|| Value::String("chat-42".to_string()));
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

        let state = TelegramServerState::default();
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

    async fn wait_for_method(&self, method: &str) -> Result<CapturedTelegramRequest> {
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
                return Ok(found);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("timed out waiting for Telegram method '{method}'"));
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

#[derive(Clone)]
struct RecordingPeerDispatcher;

#[async_trait]
impl PeerRoutingDispatcher for RecordingPeerDispatcher {
    async fn dispatch(&self, event: &PeerRoutingEvent) -> HandlerOutcome<PeerDispatchSuccess> {
        HandlerOutcome::Completed(PeerDispatchSuccess {
            peer_host_id: "peer-host-1".to_string(),
            peer_agent_name: event.peer_agent_name.clone(),
            response_text: format!("echo: {}", event.prompt),
            stop_reason: "end_turn".to_string(),
        })
    }
}

#[derive(Clone)]
struct RecordingWakeHandler;

#[async_trait]
impl DeploymentWakeHandler for RecordingWakeHandler {
    async fn wake(&self, _session_id: &SessionId) -> Result<ProvisionedRuntime> {
        Ok(ProvisionedRuntime {
            runtime_key: "runtime-key-1".to_string(),
            runtime_id: "runtime-id-2".to_string(),
        })
    }
}

#[tokio::test]
async fn ts_middleware_round_trips_durable_subscriber_profiles() -> Result<()> {
    let webhook_server = TestWebhookServer::spawn().await?;
    let telegram_server = TestTelegramServer::spawn().await?;
    let topology = render_ts_topology(&webhook_server.url)?;

    assert_topology_inventory(&topology)?;

    let stream_server = stream_server::TestStreamServer::spawn().await?;

    let result = async {
        auto_approve_round_trip(&stream_server).await?;
        webhook_round_trip(&topology, &stream_server, &webhook_server).await?;
        telegram_round_trip(&topology, &stream_server, &telegram_server).await?;
        peer_routing_round_trip(&topology, &stream_server).await?;
        wake_deployment_round_trip(&topology, &stream_server).await?;
        Ok(())
    }
    .await;

    telegram_server.shutdown().await;
    webhook_server.shutdown().await;
    stream_server.shutdown().await;

    result
}

fn render_ts_topology(webhook_url: &str) -> Result<RenderedTopology> {
    let repo_root = repo_root();
    let build_status = Command::new("pnpm")
        .args(["--filter", "@fireline/client", "build"])
        .current_dir(&repo_root)
        .status()
        .context("build @fireline/client before DS integration render")?;
    if !build_status.success() {
        return Err(anyhow!("@fireline/client build failed"));
    }

    let output = Command::new("node")
        .arg("packages/client/test/fixtures/render-ds-topology.mjs")
        .current_dir(&repo_root)
        .env("WEBHOOK_URL", webhook_url)
        .output()
        .context("render DS topology from the built TS client")?;
    if !output.status.success() {
        return Err(anyhow!(
            "render-ds-topology.mjs failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    serde_json::from_slice::<RenderedTopology>(&output.stdout)
        .context("decode rendered TS DS topology JSON")
}

fn assert_topology_inventory(topology: &RenderedTopology) -> Result<()> {
    let names: Vec<&str> = topology
        .components
        .iter()
        .map(|component| component.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec![
            "auto_approve",
            "webhook_subscriber",
            "telegram",
            "peer_routing",
            "always_on_deployment",
        ],
        "TS middleware lowering should preserve the five-profile DurableSubscriber inventory",
    );
    Ok(())
}

async fn auto_approve_round_trip(stream_server: &stream_server::TestStreamServer) -> Result<()> {
    let state_stream_url = stream_server.stream_url(&format!("auto-approve-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&state_stream_url).await?;

    let subscriber = AutoApproveSubscriber::new(AutoApproveConfig::default());
    let event = permission_request_event()?;
    let request = subscriber
        .matches(&event)
        .context("auto-approve should match permission_request")?;

    let completion = match subscriber.handle(request).await {
        HandlerOutcome::Completed(completion) => completion,
        HandlerOutcome::RetryTransient(error) | HandlerOutcome::Failed(error) => {
            return Err(error).context("auto-approve should resolve permission_request");
        }
    };
    let completion_value = serde_json::to_value(&completion)?;
    assert_eq!(
        completion_value
            .get("_meta")
            .and_then(|meta| meta.get("traceparent"))
            .and_then(Value::as_str),
        Some(trace_context().traceparent.as_deref().unwrap_or_default()),
        "auto-approve completion should preserve canonical trace lineage",
    );

    let producer = json_producer(&state_stream_url, "auto-approve");
    producer.append_json(&fireline_harness::approval::approval_resolution_envelope_with_trace(
        SessionId::from("session-1"),
        RequestId::from("req-1".to_string()),
        true,
        "auto_approve".to_string(),
        Some(trace_context()),
    )?);
    producer.flush().await?;

    let stored = read_first_envelope(&state_stream_url).await?;
    assert_eq!(stored.entity_type, "permission");
    assert_eq!(stored.completion_key(), Some(CompletionKey::prompt(
        SessionId::from("session-1"),
        RequestId::from("req-1".to_string()),
    )));
    assert_traceparent(&stored, "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01");
    Ok(())
}

async fn webhook_round_trip(
    topology: &RenderedTopology,
    stream_server: &stream_server::TestStreamServer,
    webhook_server: &TestWebhookServer,
) -> Result<()> {
    let state_stream_url = stream_server.stream_url(&format!("webhook-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&state_stream_url).await?;

    let wire: WebhookComponentConfig = serde_json::from_value(
        topology
            .component("webhook_subscriber")?
            .config
            .clone()
            .context("webhook_subscriber config")?,
    )
    .context("decode webhook_subscriber TS config")?;
    assert_eq!(wire.target_config.url, webhook_server.url);

    let subscriber = WebhookSubscriber::new(WebhookSubscriberConfig {
        target: wire.target.clone(),
        events: wire
            .events
            .into_iter()
            .map(convert_webhook_selector)
            .collect(),
        target_config: WebhookTargetConfig {
            url: wire.target_config.url,
            headers: wire.target_config.headers,
            timeout_ms: wire.target_config.timeout_ms,
            max_attempts: wire.target_config.max_attempts,
            cursor_stream: wire.target_config.cursor_stream,
            dead_letter_stream: wire.target_config.dead_letter_stream,
        },
        source_stream_url: Some(state_stream_url.clone()),
        retry_policy: wire.retry_policy.map(convert_retry_policy),
    });

    let completion = match subscriber.handle(prompt_event_with_offset()).await {
        HandlerOutcome::Completed(completion) => completion,
        HandlerOutcome::RetryTransient(error) | HandlerOutcome::Failed(error) => {
            return Err(error).context("webhook subscriber should deliver matching event");
        }
    };

    let captured = webhook_server.captured().await;
    assert_eq!(captured.len(), 1);
    assert_eq!(
        captured[0]
            .headers
            .get("traceparent")
            .and_then(|value| value.to_str().ok()),
        Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"),
        "webhook delivery should propagate W3C trace headers",
    );
    assert_eq!(
        captured[0]
            .payload
            .get("event")
            .and_then(|event| event.get("value"))
            .and_then(|value| value.get("_meta"))
            .and_then(|meta| meta.get("traceparent"))
            .and_then(Value::as_str),
        Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"),
        "webhook payload should mirror canonical trace lineage into payload _meta",
    );

    let producer = json_producer(&state_stream_url, "webhook");
    append_webhook_completion(&producer, &completion).await?;
    let stored = read_first_envelope(&state_stream_url).await?;
    assert_eq!(stored.entity_type, "webhook_delivery");
    assert_eq!(stored.completion_key(), Some(CompletionKey::prompt(
        SessionId::from("session-1"),
        RequestId::from("req-1".to_string()),
    )));
    assert_traceparent(&stored, "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01");
    Ok(())
}

async fn telegram_round_trip(
    topology: &RenderedTopology,
    stream_server: &stream_server::TestStreamServer,
    telegram_server: &TestTelegramServer,
) -> Result<()> {
    let state_stream_url = stream_server.stream_url(&format!("telegram-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&state_stream_url).await?;

    let wire: TelegramComponentConfig = serde_json::from_value(
        topology
            .component("telegram")?
            .config
            .clone()
            .context("telegram config")?,
    )
    .context("decode telegram TS config")?;
    let bot_token = wire
        .bot_token
        .context("telegram botToken should lower as a string in the integration fixture")?;
    let chat_id = wire.chat_id.context("telegram chatId")?;

    let subscriber = TelegramSubscriber::new(TelegramSubscriberConfig {
        bot_token,
        api_base_url: telegram_server.base_url.clone(),
        chat_id: Some(chat_id),
        allowed_user_ids: BTreeSet::from([String::from("42")]),
        approval_timeout: Some(Duration::from_secs(2)),
        poll_interval: Duration::from_millis(5),
        poll_timeout: Duration::ZERO,
        parse_mode: TelegramParseMode::Html,
        scope: TelegramScope::ToolCalls,
    });

    let request = subscriber
        .matches(&permission_request_event()?)
        .context("telegram subscriber should match permission_request")?;
    let subscriber_task = {
        let subscriber = subscriber.clone();
        tokio::spawn(async move { subscriber.handle(request).await })
    };

    let send_message = telegram_server.wait_for_method("sendMessage").await?;
    assert_eq!(
        send_message
            .headers
            .get("traceparent")
            .and_then(|value| value.to_str().ok()),
        Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"),
        "telegram delivery should propagate W3C trace headers",
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
        .map_err(|error| anyhow!("telegram subscriber task panicked: {error}"))?
    {
        HandlerOutcome::Completed(completion) => completion,
        HandlerOutcome::RetryTransient(error) | HandlerOutcome::Failed(error) => {
            return Err(error).context("telegram subscriber should resolve approval");
        }
    };

    let producer = json_producer(&state_stream_url, "telegram");
    append_telegram_approval_resolution(&producer, &completion).await?;
    let stored = read_first_envelope(&state_stream_url).await?;
    assert_eq!(stored.entity_type, "permission");
    assert_eq!(stored.completion_key(), Some(CompletionKey::prompt(
        SessionId::from("session-1"),
        RequestId::from("req-1".to_string()),
    )));
    assert_traceparent(&stored, "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01");

    let methods = telegram_server.captured_methods().await;
    assert!(
        methods.iter().any(|method| method == "answerCallbackQuery")
            && methods.iter().any(|method| method == "editMessageText"),
        "telegram approval tap should acknowledge the callback and edit the approval card",
    );
    Ok(())
}

async fn peer_routing_round_trip(
    topology: &RenderedTopology,
    stream_server: &stream_server::TestStreamServer,
) -> Result<()> {
    let state_stream_url = stream_server.stream_url(&format!("peer-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&state_stream_url).await?;
    let _ = topology.component("peer_routing")?;

    let subscriber = PeerRoutingSubscriber::new(Arc::new(RecordingPeerDispatcher));
    let event = prompt_peer_envelope();
    let request = subscriber
        .matches(&event)
        .context("peer_routing should match prompt_peer tool-call envelope")?;
    let completion = match subscriber.handle(request).await {
        HandlerOutcome::Completed(completion) => completion,
        HandlerOutcome::RetryTransient(error) | HandlerOutcome::Failed(error) => {
            return Err(error).context("peer_routing should complete caller-local delivery");
        }
    };

    let producer = json_producer(&state_stream_url, "peer-routing");
    producer.append_json(&json!({
        "type": PEER_DELIVERY_ACK_ENTITY_TYPE,
        "key": format!("{}:{}", completion.session_id, completion.tool_call_id),
        "headers": { "operation": "insert" },
        "value": completion,
    }));
    producer.flush().await?;

    let stored = read_first_envelope(&state_stream_url).await?;
    assert_eq!(stored.entity_type, PEER_DELIVERY_ACK_ENTITY_TYPE);
    assert_eq!(stored.completion_key(), Some(CompletionKey::tool(
        SessionId::from("session-a"),
        ToolCallId::from("tool-1".to_string()),
    )));
    assert_traceparent(&stored, "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01");
    Ok(())
}

async fn wake_deployment_round_trip(
    topology: &RenderedTopology,
    stream_server: &stream_server::TestStreamServer,
) -> Result<()> {
    let state_stream_url = stream_server.stream_url(&format!("wake-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&state_stream_url).await?;
    let _ = topology.component("always_on_deployment")?;

    let subscriber = AlwaysOnDeploymentSubscriber::with_wake_handler(RecordingWakeHandler);
    let event = DeploymentWakeRequested::new(SessionId::from("deployment-session"))
        .with_trace_context(trace_context());
    let completion = match subscriber.handle(event).await {
        HandlerOutcome::Completed(completion) => completion,
        HandlerOutcome::RetryTransient(error) | HandlerOutcome::Failed(error) => {
            return Err(error).context("wakeDeployment should reprovision deployment identity");
        }
    };

    let producer = json_producer(&state_stream_url, "wake-deployment");
    producer.append_json(&json!({
        "type": "deployment",
        "key": "deployment-session:ready",
        "headers": { "operation": "insert" },
        "value": completion,
    }));
    producer.flush().await?;

    let stored = read_first_envelope(&state_stream_url).await?;
    assert_eq!(stored.entity_type, "deployment");
    assert_eq!(stored.completion_key(), Some(CompletionKey::session(
        SessionId::from("deployment-session"),
    )));
    assert_traceparent(&stored, "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01");
    Ok(())
}

fn convert_webhook_selector(selector: WebhookEventSelectorWire) -> WebhookEventSelector {
    match selector {
        WebhookEventSelectorWire::Kind { kind } => WebhookEventSelector::Kind(kind),
        WebhookEventSelectorWire::Exact { exact } => WebhookEventSelector::Exact {
            entity_type: exact.entity_type,
            kind: exact.kind,
        },
    }
}

fn convert_retry_policy(retry: RetryPolicyWire) -> fireline_harness::RetryPolicy {
    fireline_harness::RetryPolicy {
        max_attempts: retry.max_attempts.unwrap_or(1),
        initial_backoff: Duration::from_millis(retry.initial_backoff_ms.unwrap_or(0)),
        max_backoff: Duration::from_millis(
            retry
                .max_backoff_ms
                .unwrap_or_else(|| retry.initial_backoff_ms.unwrap_or(0)),
        ),
    }
}

fn permission_request_event() -> Result<StreamEnvelope> {
    StreamEnvelope::from_json(json!({
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
    .context("decode permission_request event")
}

fn prompt_event_with_offset() -> StreamEnvelope {
    permission_request_event()
        .expect("permission_request event")
        .with_source_offset(Offset::at("0000000000000001_0000000000000000"))
}

fn prompt_peer_envelope() -> StreamEnvelope {
    let mut meta = Map::new();
    meta.insert(
        "traceparent".to_string(),
        Value::String("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string()),
    );
    meta.insert(
        "tracestate".to_string(),
        Value::String("vendor=value".to_string()),
    );
    meta.insert(
        "baggage".to_string(),
        Value::String("tenant=acme".to_string()),
    );
    let update = serde_json::to_value(SessionUpdate::ToolCall(
        ToolCall::new("tool-1", "Prompt peer")
            .raw_input(json!({
                "server": "fireline-peer",
                "tool": "prompt_peer",
                "params": {
                    "agentName": "agent-b",
                    "prompt": "hello across mesh",
                }
            }))
            .meta(meta),
    ))
    .expect("serialize peer prompt tool-call update");

    StreamEnvelope::from_json(json!({
        "type": "chunk_v2",
        "key": "session-a:req-1:tool-1:0",
        "headers": { "operation": "insert" },
        "value": {
            "sessionId": "session-a",
            "requestId": "req-1",
            "toolCallId": "tool-1",
            "update": update,
            "createdAt": 123
        }
    }))
    .expect("decode prompt_peer stream envelope")
}

fn trace_context() -> TraceContext {
    TraceContext {
        traceparent: Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string()),
        tracestate: Some("vendor=value".to_string()),
        baggage: Some("tenant=acme".to_string()),
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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

async fn read_first_envelope(stream_url: &str) -> Result<StreamEnvelope> {
    let rows = read_all_rows(stream_url).await?;
    let row = rows
        .into_iter()
        .next()
        .with_context(|| format!("expected at least one row in '{stream_url}'"))?;
    StreamEnvelope::from_json(row).context("decode stored completion envelope")
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

fn assert_traceparent(envelope: &StreamEnvelope, expected: &str) {
    assert_eq!(
        envelope
            .trace_context()
            .and_then(|trace| trace.traceparent),
        Some(expected.to_string()),
        "completion envelope should preserve canonical trace lineage",
    );
}
