use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use durable_streams::{
    Client as DurableStreamsClient, CreateOptions, LiveMode, Offset, Producer, StreamError,
};
use fireline_acp_ids::{RequestId, SessionId, ToolCallId};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::durable_subscriber::{
    ActiveSubscriber, CompletionKey, DurableSubscriber, HandlerOutcome, RetryPolicy,
    StreamEnvelope, TraceContext,
};

const WEBHOOK_COMPLETION_KIND: &str = "webhook_delivered";
const WEBHOOK_COMPLETION_TYPE: &str = "webhook_delivery";
const WEBHOOK_CURSOR_TYPE: &str = "webhook_cursor";
const WEBHOOK_DEAD_LETTER_TYPE: &str = "webhook_dead_letter";
const WEBHOOK_SUBSCRIBER_NAME: &str = "webhook_subscriber";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookSubscriberConfig {
    pub target: String,
    pub events: Vec<WebhookEventSelector>,
    pub target_config: WebhookTargetConfig,
    pub source_stream_url: Option<String>,
    pub retry_policy: Option<RetryPolicy>,
}

impl WebhookSubscriberConfig {
    #[must_use]
    pub fn retry_policy(&self) -> RetryPolicy {
        self.retry_policy.unwrap_or_else(|| RetryPolicy {
            max_attempts: self.target_config.max_attempts.max(1),
            initial_backoff: Duration::from_millis(250),
            max_backoff: Duration::from_secs(5),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookTargetConfig {
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub timeout_ms: u64,
    pub max_attempts: u32,
    pub cursor_stream: String,
    pub dead_letter_stream: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WebhookSubscriber {
    config: WebhookSubscriberConfig,
    http_client: reqwest::Client,
}

impl WebhookSubscriber {
    #[must_use]
    pub fn new(config: WebhookSubscriberConfig) -> Self {
        Self {
            config,
            http_client: reqwest::Client::new(),
        }
    }

    #[must_use]
    pub fn config(&self) -> &WebhookSubscriberConfig {
        &self.config
    }

    pub async fn dispatch_with_retry(
        &self,
        envelope: StreamEnvelope,
        agent_log: &[StreamEnvelope],
        cursor_store: &dyn WebhookCursorStore,
        dead_letters: Option<&dyn WebhookDeadLetterSink>,
    ) -> Result<WebhookDispatchResult> {
        let Some(event) = self.matches(&envelope) else {
            return Ok(WebhookDispatchResult::Skipped(
                WebhookSkipReason::SelectorMiss,
            ));
        };

        if self.is_completed(&event, agent_log) {
            return Ok(WebhookDispatchResult::Skipped(
                WebhookSkipReason::AlreadyCompleted,
            ));
        }

        let completion_key = self.completion_key(&event);
        if let Some(dead_letters) = dead_letters {
            if dead_letters
                .contains(&self.config.target, &completion_key)
                .await
                .context("check webhook dead-letter state")?
            {
                return Ok(WebhookDispatchResult::Skipped(
                    WebhookSkipReason::AlreadyDeadLettered,
                ));
            }
        }

        let source_offset = event
            .offset()
            .context("webhook dispatch requires source_offset on the matched stream envelope")?;
        if cursor_is_at_or_ahead(
            cursor_store
                .load_cursor()
                .await
                .context("load webhook delivery cursor")?
                .as_ref(),
            &source_offset,
        ) {
            return Ok(WebhookDispatchResult::Skipped(
                WebhookSkipReason::AlreadyAcknowledged,
            ));
        }

        let mut attempts = 0;
        loop {
            attempts += 1;
            match self.handle(event.clone()).await {
                HandlerOutcome::Completed(completion) => {
                    return Ok(WebhookDispatchResult::Delivered(WebhookDispatchOutcome {
                        completion,
                        source_offset,
                        attempts,
                    }));
                }
                HandlerOutcome::RetryTransient(error) => {
                    if let Some(backoff) = self.retry_policy().backoff_for_attempt(attempts) {
                        tokio::time::sleep(backoff).await;
                        continue;
                    }

                    let record = WebhookDeadLetterRecord::from_error(
                        &self.config.target,
                        &event,
                        source_offset.to_string(),
                        attempts,
                        error,
                    )?;
                    if let Some(dead_letters) = dead_letters {
                        dead_letters
                            .record(&record)
                            .await
                            .context("record webhook dead-letter")?;
                    }
                    return Ok(WebhookDispatchResult::DeadLettered(record));
                }
                HandlerOutcome::Failed(error) => {
                    let record = WebhookDeadLetterRecord::from_error(
                        &self.config.target,
                        &event,
                        source_offset.to_string(),
                        attempts,
                        error,
                    )?;
                    if let Some(dead_letters) = dead_letters {
                        dead_letters
                            .record(&record)
                            .await
                            .context("record webhook dead-letter")?;
                    }
                    return Ok(WebhookDispatchResult::DeadLettered(record));
                }
            }
        }
    }

    async fn dispatch_once(&self, event: StreamEnvelope) -> HandlerOutcome<WebhookDelivered> {
        let source_offset = match event.offset() {
            Some(offset) => offset,
            None => {
                return HandlerOutcome::Failed(anyhow!(
                    "webhook dispatch requires source_offset on the matched stream envelope"
                ));
            }
        };

        let trace = event.trace_context().unwrap_or_default();
        let payload = WebhookDeliveryPayload {
            subscription: self.config.target.clone(),
            stream_url: self.config.source_stream_url.clone(),
            offset: source_offset.to_string(),
            delivered_at_ms: now_ms(),
            meta: (!trace.is_empty()).then_some(trace.clone()),
            event: event.clone().without_source_offset(),
        };

        let mut request = self
            .http_client
            .post(&self.config.target_config.url)
            .timeout(Duration::from_millis(self.config.target_config.timeout_ms))
            .header("content-type", "application/json");

        for (name, value) in &self.config.target_config.headers {
            request = request.header(name.as_str(), value.as_str());
        }
        request = apply_trace_headers(request, &trace);

        let response = match request.json(&payload).send().await {
            Ok(response) => response,
            Err(error) => return HandlerOutcome::RetryTransient(anyhow!(error)),
        };
        let status = response.status();

        if status.is_success() {
            match WebhookDelivered::from_event(&self.config.target, &event, status.as_u16()) {
                Ok(completion) => HandlerOutcome::Completed(completion),
                Err(error) => HandlerOutcome::Failed(error),
            }
        } else if status == StatusCode::REQUEST_TIMEOUT
            || status == StatusCode::TOO_MANY_REQUESTS
            || status.is_server_error()
        {
            HandlerOutcome::RetryTransient(anyhow!(
                "webhook target '{}' returned transient HTTP {}",
                self.config.target,
                status
            ))
        } else {
            HandlerOutcome::Failed(anyhow!(
                "webhook target '{}' returned terminal HTTP {}",
                self.config.target,
                status
            ))
        }
    }
}

impl DurableSubscriber for WebhookSubscriber {
    type Event = StreamEnvelope;
    type Completion = WebhookDelivered;

    fn name(&self) -> &str {
        WEBHOOK_SUBSCRIBER_NAME
    }

    fn matches(&self, envelope: &StreamEnvelope) -> Option<Self::Event> {
        if envelope.completion_key().is_none() {
            return None;
        }

        self.config
            .events
            .iter()
            .any(|selector| selector.matches(envelope))
            .then(|| envelope.clone())
    }

    fn completion_key(&self, event: &Self::Event) -> CompletionKey {
        event
            .completion_key()
            .expect("webhook subscriber only matches envelopes with canonical completion keys")
    }

    fn is_completed(&self, event: &Self::Event, log: &[StreamEnvelope]) -> bool {
        let completion_key = self.completion_key(event);
        log.iter().any(|entry| {
            entry
                .value_as::<WebhookDelivered>()
                .and_then(|completion| {
                    (completion.kind == WEBHOOK_COMPLETION_KIND
                        && completion.target == self.config.target)
                        .then_some(completion)
                })
                .and_then(|completion| completion.completion_key())
                .is_some_and(|observed| observed == completion_key)
        })
    }
}

#[async_trait]
impl ActiveSubscriber for WebhookSubscriber {
    async fn handle(&self, event: Self::Event) -> HandlerOutcome<Self::Completion> {
        self.dispatch_once(event).await
    }

    fn retry_policy(&self) -> RetryPolicy {
        self.config.retry_policy()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookEventSelector {
    Kind(String),
    Exact {
        entity_type: String,
        kind: Option<String>,
    },
}

impl WebhookEventSelector {
    #[must_use]
    pub fn matches(&self, envelope: &StreamEnvelope) -> bool {
        let value_kind = envelope
            .value
            .as_ref()
            .and_then(|value| value.get("kind"))
            .and_then(Value::as_str);

        match self {
            Self::Kind(needle) => {
                value_kind == Some(needle.as_str()) || envelope.entity_type == *needle
            }
            Self::Exact { entity_type, kind } => {
                envelope.entity_type == *entity_type
                    && kind
                        .as_deref()
                        .is_none_or(|expected| value_kind == Some(expected))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WebhookDeliveryPayload {
    pub subscription: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_url: Option<String>,
    pub offset: String,
    pub delivered_at_ms: i64,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<TraceContext>,
    pub event: StreamEnvelope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebhookDelivered {
    #[serde(default = "webhook_completion_kind")]
    pub kind: String,
    pub target: String,
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<RequestId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<ToolCallId>,
    pub offset: String,
    pub delivered_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<TraceContext>,
}

impl WebhookDelivered {
    fn from_event(target: &str, event: &StreamEnvelope, status_code: u16) -> Result<Self> {
        let completion_key = event
            .completion_key()
            .context("webhook completion requires a canonical completion key")?;
        let trace = event.trace_context();
        let offset = event
            .offset()
            .context("webhook completion requires source_offset")?
            .to_string();

        Ok(match completion_key {
            CompletionKey::Prompt {
                session_id,
                request_id,
            } => Self {
                kind: webhook_completion_kind(),
                target: target.to_string(),
                session_id,
                request_id: Some(request_id),
                tool_call_id: None,
                offset,
                delivered_at_ms: now_ms(),
                status_code: Some(status_code),
                meta: trace,
            },
            CompletionKey::Tool {
                session_id,
                tool_call_id,
            } => Self {
                kind: webhook_completion_kind(),
                target: target.to_string(),
                session_id,
                request_id: None,
                tool_call_id: Some(tool_call_id),
                offset,
                delivered_at_ms: now_ms(),
                status_code: Some(status_code),
                meta: trace,
            },
            CompletionKey::Session { session_id } => Self {
                kind: webhook_completion_kind(),
                target: target.to_string(),
                session_id,
                request_id: None,
                tool_call_id: None,
                offset,
                delivered_at_ms: now_ms(),
                status_code: Some(status_code),
                meta: trace,
            },
        })
    }

    #[must_use]
    pub fn completion_key(&self) -> Option<CompletionKey> {
        match (&self.request_id, &self.tool_call_id) {
            (Some(request_id), None) => Some(CompletionKey::prompt(
                self.session_id.clone(),
                request_id.clone(),
            )),
            (None, Some(tool_call_id)) => Some(CompletionKey::tool(
                self.session_id.clone(),
                tool_call_id.clone(),
            )),
            (None, None) => Some(CompletionKey::session(self.session_id.clone())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebhookCursorRecord {
    pub target: String,
    pub offset: String,
    pub acknowledged_at_ms: i64,
}

impl WebhookCursorRecord {
    #[must_use]
    pub fn new(target: impl Into<String>, offset: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            offset: offset.into(),
            acknowledged_at_ms: now_ms(),
        }
    }

    #[must_use]
    pub fn offset(&self) -> Offset {
        Offset::parse(&self.offset)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebhookDeadLetterRecord {
    pub target: String,
    pub completion_key: CompletionKey,
    pub offset: String,
    pub attempts: u32,
    pub last_error: String,
    pub created_at_ms: i64,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<TraceContext>,
}

impl WebhookDeadLetterRecord {
    fn from_error(
        target: &str,
        event: &StreamEnvelope,
        offset: String,
        attempts: u32,
        error: anyhow::Error,
    ) -> Result<Self> {
        Ok(Self {
            target: target.to_string(),
            completion_key: event
                .completion_key()
                .context("webhook dead-letter requires a canonical completion key")?,
            offset,
            attempts,
            last_error: error.to_string(),
            created_at_ms: now_ms(),
            meta: event.trace_context(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookDispatchOutcome {
    pub completion: WebhookDelivered,
    pub source_offset: Offset,
    pub attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookDispatchResult {
    Skipped(WebhookSkipReason),
    Delivered(WebhookDispatchOutcome),
    DeadLettered(WebhookDeadLetterRecord),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookSkipReason {
    SelectorMiss,
    AlreadyCompleted,
    AlreadyAcknowledged,
    AlreadyDeadLettered,
}

#[async_trait]
pub trait WebhookCursorStore: Send + Sync {
    async fn load_cursor(&self) -> Result<Option<Offset>>;
    async fn acknowledge_cursor(&self, record: &WebhookCursorRecord) -> Result<bool>;
}

#[async_trait]
pub trait WebhookDeadLetterSink: Send + Sync {
    async fn contains(&self, target: &str, completion_key: &CompletionKey) -> Result<bool>;
    async fn record(&self, record: &WebhookDeadLetterRecord) -> Result<()>;
}

#[derive(Debug)]
pub struct DurableWebhookCursorStore {
    stream_url: String,
    cached_offset: Mutex<Option<Offset>>,
}

impl DurableWebhookCursorStore {
    #[must_use]
    pub fn new(stream_url: impl Into<String>) -> Self {
        Self {
            stream_url: stream_url.into(),
            cached_offset: Mutex::new(None),
        }
    }

    async fn replay_cursor(&self) -> Result<Option<Offset>> {
        let Some(rows) = read_all_json_rows(&self.stream_url).await? else {
            return Ok(None);
        };

        let mut latest = None;
        for row in rows {
            let Ok(envelope) = serde_json::from_value::<StateEnvelope<WebhookCursorRecord>>(row)
            else {
                continue;
            };
            if envelope.entity_type != WEBHOOK_CURSOR_TYPE {
                continue;
            }
            let candidate = envelope.value.offset();
            if offset_is_newer(latest.as_ref(), &candidate) {
                latest = Some(candidate);
            }
        }
        Ok(latest)
    }
}

#[async_trait]
impl WebhookCursorStore for DurableWebhookCursorStore {
    async fn load_cursor(&self) -> Result<Option<Offset>> {
        let mut cached = self.cached_offset.lock().await;
        if cached.is_none() {
            *cached = self.replay_cursor().await?;
        }
        Ok(cached.clone())
    }

    async fn acknowledge_cursor(&self, record: &WebhookCursorRecord) -> Result<bool> {
        let candidate = record.offset();
        let mut cached = self.cached_offset.lock().await;
        if !offset_is_newer(cached.as_ref(), &candidate) {
            return Ok(false);
        }

        ensure_json_stream_exists(&self.stream_url)
            .await
            .with_context(|| format!("ensure webhook cursor stream '{}'", self.stream_url))?;
        let producer = json_producer(&self.stream_url, "webhook-cursor");
        producer.append_json(&StateEnvelope {
            entity_type: WEBHOOK_CURSOR_TYPE.to_string(),
            key: record.target.clone(),
            headers: StateHeaders {
                operation: "insert".to_string(),
            },
            value: record.clone(),
        });
        producer
            .flush()
            .await
            .context("flush webhook cursor update")?;

        *cached = Some(candidate);
        Ok(true)
    }
}

#[derive(Debug)]
pub struct DurableWebhookDeadLetterSink {
    stream_url: String,
    known_keys: Mutex<Option<HashSet<String>>>,
}

impl DurableWebhookDeadLetterSink {
    #[must_use]
    pub fn new(stream_url: impl Into<String>) -> Self {
        Self {
            stream_url: stream_url.into(),
            known_keys: Mutex::new(None),
        }
    }

    async fn replay_keys(&self) -> Result<HashSet<String>> {
        let Some(rows) = read_all_json_rows(&self.stream_url).await? else {
            return Ok(HashSet::new());
        };

        let mut known = HashSet::new();
        for row in rows {
            let Ok(envelope) =
                serde_json::from_value::<StateEnvelope<WebhookDeadLetterRecord>>(row)
            else {
                continue;
            };
            if envelope.entity_type != WEBHOOK_DEAD_LETTER_TYPE {
                continue;
            }
            known.insert(dead_letter_storage_key(
                &envelope.value.target,
                &envelope.value.completion_key,
            ));
        }
        Ok(known)
    }
}

#[async_trait]
impl WebhookDeadLetterSink for DurableWebhookDeadLetterSink {
    async fn contains(&self, target: &str, completion_key: &CompletionKey) -> Result<bool> {
        let mut known_keys = self.known_keys.lock().await;
        if known_keys.is_none() {
            *known_keys = Some(self.replay_keys().await?);
        }
        Ok(known_keys
            .as_ref()
            .is_some_and(|keys| keys.contains(&dead_letter_storage_key(target, completion_key))))
    }

    async fn record(&self, record: &WebhookDeadLetterRecord) -> Result<()> {
        ensure_json_stream_exists(&self.stream_url)
            .await
            .with_context(|| format!("ensure webhook dead-letter stream '{}'", self.stream_url))?;
        let producer = json_producer(&self.stream_url, "webhook-dead-letter");
        producer.append_json(&StateEnvelope {
            entity_type: WEBHOOK_DEAD_LETTER_TYPE.to_string(),
            key: dead_letter_storage_key(&record.target, &record.completion_key),
            headers: StateHeaders {
                operation: "insert".to_string(),
            },
            value: record.clone(),
        });
        producer
            .flush()
            .await
            .context("flush webhook dead-letter update")?;

        let mut known_keys = self.known_keys.lock().await;
        let keys = known_keys.get_or_insert_with(HashSet::new);
        keys.insert(dead_letter_storage_key(
            &record.target,
            &record.completion_key,
        ));
        Ok(())
    }
}

pub async fn append_webhook_completion(
    producer: &Producer,
    completion: &WebhookDelivered,
) -> Result<()> {
    producer.append_json(&StateEnvelope {
        entity_type: WEBHOOK_COMPLETION_TYPE.to_string(),
        key: completion_storage_key(completion),
        headers: StateHeaders {
            operation: "insert".to_string(),
        },
        value: completion.clone(),
    });
    producer.flush().await.context("flush webhook completion")
}

fn apply_trace_headers(
    mut request: reqwest::RequestBuilder,
    trace: &TraceContext,
) -> reqwest::RequestBuilder {
    if let Some(traceparent) = trace.traceparent.as_deref() {
        request = request.header("traceparent", traceparent);
    }
    if let Some(tracestate) = trace.tracestate.as_deref() {
        request = request.header("tracestate", tracestate);
    }
    if let Some(baggage) = trace.baggage.as_deref() {
        request = request.header("baggage", baggage);
    }
    request
}

fn completion_storage_key(completion: &WebhookDelivered) -> String {
    let completion_key = completion
        .completion_key()
        .expect("webhook completion must contain exactly one canonical key shape");
    format!(
        "{}:{}:delivered",
        completion.target,
        completion_key.storage_key()
    )
}

fn cursor_is_at_or_ahead(current: Option<&Offset>, candidate: &Offset) -> bool {
    current
        .and_then(|current| current.partial_cmp(candidate))
        .is_some_and(|order| order.is_ge())
}

fn offset_is_newer(current: Option<&Offset>, candidate: &Offset) -> bool {
    match current.and_then(|current| current.partial_cmp(candidate)) {
        Some(std::cmp::Ordering::Less) => true,
        Some(std::cmp::Ordering::Equal | std::cmp::Ordering::Greater) => false,
        None => current.is_none(),
    }
}

fn dead_letter_storage_key(target: &str, completion_key: &CompletionKey) -> String {
    format!("{target}:{}", completion_key.storage_key())
}

fn webhook_completion_kind() -> String {
    WEBHOOK_COMPLETION_KIND.to_string()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as i64
}

fn json_producer(stream_url: &str, producer_name: &str) -> Producer {
    let client = DurableStreamsClient::new();
    let mut stream = client.stream(stream_url);
    stream.set_content_type("application/json");
    stream
        .producer(format!("{producer_name}-{}", uuid::Uuid::new_v4()))
        .content_type("application/json")
        .build()
}

async fn ensure_json_stream_exists(stream_url: &str) -> Result<()> {
    let client = DurableStreamsClient::new();
    let stream = client.stream(stream_url);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match stream
            .create_with(CreateOptions::new().content_type("application/json"))
            .await
        {
            Ok(_) | Err(StreamError::Conflict) => return Ok(()),
            Err(error) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                tracing::debug!(?error, stream_url, "retrying webhook test stream creation");
            }
            Err(error) => {
                return Err(anyhow::Error::from(error))
                    .with_context(|| format!("create webhook stream '{stream_url}'"));
            }
        }
    }
}

async fn read_all_json_rows(stream_url: &str) -> Result<Option<Vec<Value>>> {
    let client = DurableStreamsClient::new();
    let stream = client.stream(stream_url);
    let mut reader = match stream
        .read()
        .offset(Offset::Beginning)
        .live(LiveMode::Off)
        .build()
    {
        Ok(reader) => reader,
        Err(error) => {
            return Err(anyhow::Error::from(error)).context("build webhook stream reader");
        }
    };

    let mut rows = Vec::new();
    loop {
        match reader.next_chunk().await {
            Ok(Some(chunk)) => {
                if !chunk.data.is_empty() {
                    let chunk_rows: Vec<Value> = serde_json::from_slice(&chunk.data)
                        .context("decode webhook durable stream chunk as JSON array")?;
                    rows.extend(chunk_rows);
                }
                if chunk.up_to_date {
                    break;
                }
            }
            Ok(None) => break,
            Err(StreamError::NotFound { .. }) => return Ok(None),
            Err(error) => {
                return Err(anyhow::Error::from(error))
                    .with_context(|| format!("read webhook stream '{stream_url}'"));
            }
        }
    }
    Ok(Some(rows))
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

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;

    use anyhow::Result;
    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::post,
    };
    use tokio::sync::oneshot;

    use super::*;

    #[derive(Debug, Clone)]
    struct CapturedRequest {
        headers: HeaderMap,
        payload: WebhookDeliveryPayload,
    }

    #[derive(Clone, Default)]
    struct TestWebhookState {
        captured: Arc<Mutex<Vec<CapturedRequest>>>,
        statuses: Arc<Mutex<VecDeque<StatusCode>>>,
    }

    struct TestWebhookServer {
        url: String,
        state: TestWebhookState,
        shutdown_tx: Option<oneshot::Sender<()>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl TestWebhookServer {
        async fn spawn(statuses: impl Into<VecDeque<StatusCode>>) -> Result<Self> {
            async fn handler(
                State(state): State<TestWebhookState>,
                headers: HeaderMap,
                Json(payload): Json<WebhookDeliveryPayload>,
            ) -> StatusCode {
                state
                    .captured
                    .lock()
                    .await
                    .push(CapturedRequest { headers, payload });
                state
                    .statuses
                    .lock()
                    .await
                    .pop_front()
                    .unwrap_or(StatusCode::OK)
            }

            let state = TestWebhookState {
                captured: Arc::new(Mutex::new(Vec::new())),
                statuses: Arc::new(Mutex::new(statuses.into())),
            };
            let router = Router::new()
                .route("/hook", post(handler))
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
                url: format!("http://127.0.0.1:{}/hook", addr.port()),
                state,
                shutdown_tx: Some(shutdown_tx),
                task,
            })
        }

        async fn captured(&self) -> Vec<CapturedRequest> {
            self.state.captured.lock().await.clone()
        }

        async fn shutdown(mut self) {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(());
            }
            let _ = self.task.await;
        }
    }

    struct TestStreamServer {
        base_url: String,
        shutdown_tx: Option<oneshot::Sender<()>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl TestStreamServer {
        async fn spawn() -> Result<Self> {
            let router: Router = fireline_session::build_stream_router(None)?;
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
                base_url: format!("http://127.0.0.1:{}/v1/stream", addr.port()),
                shutdown_tx: Some(shutdown_tx),
                task,
            })
        }

        fn stream_url(&self, name: &str) -> String {
            format!("{}/{}", self.base_url.trim_end_matches('/'), name)
        }

        async fn shutdown(mut self) {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(());
            }
            let _ = self.task.await;
        }
    }

    #[derive(Debug, Default)]
    struct MemoryCursorStore {
        offset: Mutex<Option<Offset>>,
    }

    #[async_trait]
    impl WebhookCursorStore for MemoryCursorStore {
        async fn load_cursor(&self) -> Result<Option<Offset>> {
            Ok(self.offset.lock().await.clone())
        }

        async fn acknowledge_cursor(&self, record: &WebhookCursorRecord) -> Result<bool> {
            let candidate = record.offset();
            let mut offset = self.offset.lock().await;
            if !offset_is_newer(offset.as_ref(), &candidate) {
                return Ok(false);
            }
            *offset = Some(candidate);
            Ok(true)
        }
    }

    #[derive(Debug, Default)]
    struct MemoryDeadLetters {
        records: Mutex<Vec<WebhookDeadLetterRecord>>,
    }

    #[async_trait]
    impl WebhookDeadLetterSink for MemoryDeadLetters {
        async fn contains(&self, target: &str, completion_key: &CompletionKey) -> Result<bool> {
            Ok(self
                .records
                .lock()
                .await
                .iter()
                .any(|record| record.target == target && record.completion_key == *completion_key))
        }

        async fn record(&self, record: &WebhookDeadLetterRecord) -> Result<()> {
            self.records.lock().await.push(record.clone());
            Ok(())
        }
    }

    fn subscriber_config(url: String) -> WebhookSubscriberConfig {
        WebhookSubscriberConfig {
            target: "slack-approvals".to_string(),
            events: vec![WebhookEventSelector::Kind("permission_request".to_string())],
            target_config: WebhookTargetConfig {
                url,
                headers: BTreeMap::from([(
                    "x-fireline-source".to_string(),
                    "approval-gate".to_string(),
                )]),
                timeout_ms: 1_000,
                max_attempts: 3,
                cursor_stream: "unused".to_string(),
                dead_letter_stream: None,
            },
            source_stream_url: Some("http://streams/state/session-1".to_string()),
            retry_policy: Some(RetryPolicy {
                max_attempts: 3,
                initial_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(5),
            }),
        }
    }

    fn prompt_event() -> StreamEnvelope {
        StreamEnvelope::from_json(serde_json::json!({
            "type": "permission",
            "key": "session-1:req-1",
            "headers": { "operation": "insert" },
            "value": {
                "kind": "permission_request",
                "sessionId": "session-1",
                "requestId": "req-1",
                "reason": "approval required",
                "_meta": {
                    "traceparent": "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01",
                    "tracestate": "vendor=value",
                    "baggage": "tenant=acme"
                }
            }
        }))
        .expect("decode prompt webhook event")
        .with_source_offset(Offset::at("0000000000000001_0000000000000000"))
    }

    #[test]
    fn selector_matches_kind_before_falling_back_to_type() {
        let event = prompt_event();
        assert!(WebhookEventSelector::Kind("permission_request".to_string()).matches(&event));
        assert!(WebhookEventSelector::Kind("permission".to_string()).matches(&event));
        assert!(
            !WebhookEventSelector::Kind("tool_call".to_string()).matches(&event),
            "DSV-12 ConcurrentResolutionIsolatedByKey: selector fallback must not match unrelated event kinds"
        );
        assert!(
            WebhookEventSelector::Exact {
                entity_type: "permission".to_string(),
                kind: Some("permission_request".to_string()),
            }
            .matches(&event)
        );
    }

    #[tokio::test]
    async fn handle_posts_payload_with_trace_headers_and_meta() -> Result<()> {
        let server = TestWebhookServer::spawn(VecDeque::from([StatusCode::OK])).await?;
        let subscriber = WebhookSubscriber::new(subscriber_config(server.url.clone()));

        let outcome = subscriber.handle(prompt_event()).await;
        let completion = match outcome {
            HandlerOutcome::Completed(completion) => completion,
            other => panic!("DSV-05 TraceContextPropagated: expected completed webhook delivery, got {other:?}"),
        };

        let captured = server.captured().await;
        assert_eq!(captured.len(), 1);
        let request = &captured[0];
        assert_eq!(
            request
                .headers
                .get("traceparent")
                .and_then(|value| value.to_str().ok()),
            Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"),
            "DSV-05 TraceContextPropagated: webhook POST must carry traceparent from the source event"
        );
        assert_eq!(
            request
                .headers
                .get("tracestate")
                .and_then(|value| value.to_str().ok()),
            Some("vendor=value")
        );
        assert_eq!(
            request
                .headers
                .get("baggage")
                .and_then(|value| value.to_str().ok()),
            Some("tenant=acme")
        );
        assert_eq!(
            request
                .headers
                .get("x-fireline-source")
                .and_then(|value| value.to_str().ok()),
            Some("approval-gate")
        );
        assert_eq!(request.payload.subscription, "slack-approvals");
        assert_eq!(
            request.payload.stream_url.as_deref(),
            Some("http://streams/state/session-1")
        );
        assert_eq!(
            request
                .payload
                .meta
                .as_ref()
                .and_then(|meta| meta.traceparent.as_deref()),
            Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01")
        );
        assert_eq!(
            request
                .payload
                .event
                .trace_context()
                .and_then(|meta| meta.traceparent),
            Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string())
        );
        assert_eq!(
            completion.completion_key(),
            Some(CompletionKey::prompt(
                SessionId::from("session-1"),
                RequestId::from("req-1".to_string())
            )),
            "DSV-01 CompletionKeyUnique: webhook completion must stay on the canonical prompt key"
        );
        assert_eq!(
            completion
                .meta
                .as_ref()
                .and_then(|meta| meta.traceparent.as_deref()),
            Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01")
        );

        server.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn dispatch_with_retry_is_at_least_once_and_dead_letters_after_budget() -> Result<()> {
        let server = TestWebhookServer::spawn(VecDeque::from([
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::OK,
        ]))
        .await?;
        let subscriber = WebhookSubscriber::new(subscriber_config(server.url.clone()));
        let cursor_store = MemoryCursorStore::default();
        let dead_letters = MemoryDeadLetters::default();

        let result = subscriber
            .dispatch_with_retry(prompt_event(), &[], &cursor_store, Some(&dead_letters))
            .await?;
        let outcome = match result {
            WebhookDispatchResult::Delivered(outcome) => outcome,
            other => panic!("DSV-03 RetryBounded: expected delivered webhook dispatch, got {other:?}"),
        };
        assert_eq!(outcome.attempts, 2);
        assert_eq!(server.captured().await.len(), 2);
        assert!(dead_letters.records.lock().await.is_empty());

        let failure_server = TestWebhookServer::spawn(VecDeque::from([
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::BAD_REQUEST,
        ]))
        .await?;
        let failure_subscriber =
            WebhookSubscriber::new(subscriber_config(failure_server.url.clone()));
        let failure_dead_letters = MemoryDeadLetters::default();
        let result = failure_subscriber
            .dispatch_with_retry(
                prompt_event(),
                &[],
                &MemoryCursorStore::default(),
                Some(&failure_dead_letters),
            )
            .await?;
        let record = match result {
            WebhookDispatchResult::DeadLettered(record) => record,
            other => panic!("DSV-04 DeadLetterTerminal: expected dead-lettered webhook dispatch, got {other:?}"),
        };
        assert_eq!(record.attempts, 3);
        assert_eq!(failure_server.captured().await.len(), 3);
        assert_eq!(failure_dead_letters.records.lock().await.len(), 1);

        server.shutdown().await;
        failure_server.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn dispatch_replays_until_durable_cursor_ack_then_skips() -> Result<()> {
        let stream_server = TestStreamServer::spawn().await?;
        let cursor_stream_url =
            stream_server.stream_url(&format!("cursor-replay-{}", uuid::Uuid::new_v4()));
        let server = TestWebhookServer::spawn(VecDeque::from([
            StatusCode::OK,
            StatusCode::OK,
        ]))
        .await?;
        let subscriber = WebhookSubscriber::new(subscriber_config(server.url.clone()));

        let first_result = subscriber
            .dispatch_with_retry(
                prompt_event(),
                &[],
                &DurableWebhookCursorStore::new(cursor_stream_url.clone()),
                None,
            )
            .await?;
        let first = match first_result {
            WebhookDispatchResult::Delivered(outcome) => outcome,
            other => panic!("expected first replay attempt to deliver, got {other:?}"),
        };
        assert_eq!(first.attempts, 1);
        assert_eq!(server.captured().await.len(), 1);

        let replay_result = subscriber
            .dispatch_with_retry(
                prompt_event(),
                &[],
                &DurableWebhookCursorStore::new(cursor_stream_url.clone()),
                None,
            )
            .await?;
        let replay = match replay_result {
            WebhookDispatchResult::Delivered(outcome) => outcome,
            other => panic!("expected replay without cursor ack to redeliver, got {other:?}"),
        };
        assert_eq!(replay.attempts, 1);
        assert_eq!(
            server.captured().await.len(),
            2,
            "without persisted completion or cursor ack, replay should redeliver the same envelope"
        );

        let ack_store = DurableWebhookCursorStore::new(cursor_stream_url.clone());
        assert!(
            ack_store
                .acknowledge_cursor(&WebhookCursorRecord::new(
                    "slack-approvals",
                    replay.source_offset.to_string(),
                ))
                .await?
        );

        let skipped = subscriber
            .dispatch_with_retry(
                prompt_event(),
                &[],
                &DurableWebhookCursorStore::new(cursor_stream_url),
                None,
            )
            .await?;
        assert_eq!(
            skipped,
            WebhookDispatchResult::Skipped(WebhookSkipReason::AlreadyAcknowledged)
        );

        server.shutdown().await;
        stream_server.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn completion_append_and_cursor_ack_are_monotonic() -> Result<()> {
        let stream_server = TestStreamServer::spawn().await?;
        let state_stream_url = stream_server.stream_url(&format!("state-{}", uuid::Uuid::new_v4()));
        let cursor_stream_url =
            stream_server.stream_url(&format!("cursor-{}", uuid::Uuid::new_v4()));
        ensure_json_stream_exists(&state_stream_url).await?;

        let completion = WebhookDelivered {
            kind: webhook_completion_kind(),
            target: "slack-approvals".to_string(),
            session_id: SessionId::from("session-1"),
            request_id: Some(RequestId::from("req-1".to_string())),
            tool_call_id: None,
            offset: "0000000000000002_0000000000000000".to_string(),
            delivered_at_ms: now_ms(),
            status_code: Some(200),
            meta: Some(TraceContext {
                traceparent: Some(
                    "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string(),
                ),
                tracestate: None,
                baggage: None,
            }),
        };
        let producer = json_producer(&state_stream_url, "webhook-completion-test");
        append_webhook_completion(&producer, &completion).await?;

        let rows = read_all_json_rows(&state_stream_url)
            .await?
            .expect("state stream rows");
        let stored: StateEnvelope<WebhookDelivered> =
            serde_json::from_value(rows.into_iter().next().expect("completion row"))?;
        assert_eq!(stored.entity_type, WEBHOOK_COMPLETION_TYPE);
        assert_eq!(
            stored.key,
            "slack-approvals:prompt:session-1:req-1:delivered"
        );
        assert_eq!(stored.value, completion);

        let cursor_store = DurableWebhookCursorStore::new(cursor_stream_url.clone());
        assert!(
            cursor_store
                .acknowledge_cursor(&WebhookCursorRecord::new(
                    "slack-approvals",
                    "0000000000000002_0000000000000000"
                ))
                .await?
        );
        assert!(
            !cursor_store
                .acknowledge_cursor(&WebhookCursorRecord::new(
                    "slack-approvals",
                    "0000000000000001_0000000000000000"
                ))
                .await?,
            "cursor monotonicity requires stale offsets to be ignored"
        );
        assert_eq!(
            cursor_store.load_cursor().await?,
            Some(Offset::at("0000000000000002_0000000000000000"))
        );
        let reloaded = DurableWebhookCursorStore::new(cursor_stream_url.clone());
        assert_eq!(
            reloaded.load_cursor().await?,
            Some(Offset::at("0000000000000002_0000000000000000"))
        );
        assert!(
            !reloaded
                .acknowledge_cursor(&WebhookCursorRecord::new(
                    "slack-approvals",
                    "0000000000000002_0000000000000000"
                ))
                .await?,
            "equal offsets must not regress cursor state"
        );
        assert!(
            reloaded
                .acknowledge_cursor(&WebhookCursorRecord::new(
                    "slack-approvals",
                    "0000000000000003_0000000000000000"
                ))
                .await?,
            "a newer offset should advance the durable cursor after reload"
        );
        let advanced = DurableWebhookCursorStore::new(cursor_stream_url);
        assert_eq!(
            advanced.load_cursor().await?,
            Some(Offset::at("0000000000000003_0000000000000000"))
        );

        stream_server.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn replay_skips_duplicate_webhook_post_when_completion_already_exists() -> Result<()> {
        let server = TestWebhookServer::spawn(VecDeque::from([StatusCode::OK])).await?;
        let subscriber = WebhookSubscriber::new(subscriber_config(server.url.clone()));
        let mut driver = crate::DurableSubscriberDriver::new();
        driver.register_active(subscriber.clone());
        assert_eq!(
            driver.registrations(),
            vec![crate::SubscriberRegistration {
                name: "webhook_subscriber".to_string(),
                mode: crate::SubscriberMode::Active,
            }],
            "DSV-02 ReplayIdempotent: a fresh driver must register the webhook_subscriber profile before replay suppression is evaluated",
        );

        let event = prompt_event();
        let matched = subscriber
            .matches(&event)
            .expect("DSV-02 ReplayIdempotent: webhook should still match the replayed permission_request");
        let completion = WebhookDelivered::from_event("slack-approvals", &event, 200)?;
        let replay_log = vec![StreamEnvelope::from_json(serde_json::json!({
            "type": WEBHOOK_COMPLETION_TYPE,
            "key": "slack-approvals:prompt:session-1:req-1:delivered",
            "headers": { "operation": "insert" },
            "value": completion,
        }))
        .expect("decode webhook completion envelope")];

        assert!(
            subscriber.is_completed(&matched, &replay_log),
            "DSV-02 ReplayIdempotent: replay with a preexisting webhook_delivered completion must mark the event complete",
        );
        let should_dispatch = !subscriber.is_completed(&matched, &replay_log);
        assert!(
            !should_dispatch,
            "DSV-02 ReplayIdempotent: replay must skip a duplicate webhook POST when webhook_delivered already exists",
        );
        assert_eq!(
            server.captured().await.len(),
            0,
            "DSV-02 ReplayIdempotent: replay suppression must keep webhook side effects at zero",
        );

        server.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn completion_envelope_and_dead_letter_gate_future_replay() -> Result<()> {
        let server = TestWebhookServer::spawn(VecDeque::from([StatusCode::OK])).await?;
        let subscriber = WebhookSubscriber::new(subscriber_config(server.url.clone()));
        let event = prompt_event();
        let completion = WebhookDelivered::from_event("slack-approvals", &event, 200)?;
        let completion_log = vec![StreamEnvelope::from_json(serde_json::json!({
            "type": WEBHOOK_COMPLETION_TYPE,
            "key": completion_storage_key(&completion),
            "headers": { "operation": "insert" },
            "value": completion
        }))?];
        let skipped = subscriber
            .dispatch_with_retry(
                event.clone(),
                &completion_log,
                &MemoryCursorStore::default(),
                Some(&MemoryDeadLetters::default()),
            )
            .await?;
        assert_eq!(
            skipped,
            WebhookDispatchResult::Skipped(WebhookSkipReason::AlreadyCompleted)
        );

        let dead_letters = MemoryDeadLetters::default();
        dead_letters
            .record(&WebhookDeadLetterRecord {
                target: "slack-approvals".to_string(),
                completion_key: event.completion_key().expect("canonical key"),
                offset: event.offset().expect("source offset").to_string(),
                attempts: 3,
                last_error: "terminal".to_string(),
                created_at_ms: now_ms(),
                meta: event.trace_context(),
            })
            .await?;
        let skipped = subscriber
            .dispatch_with_retry(
                event,
                &[],
                &MemoryCursorStore::default(),
                Some(&dead_letters),
            )
            .await?;
        assert_eq!(
            skipped,
            WebhookDispatchResult::Skipped(WebhookSkipReason::AlreadyDeadLettered)
        );

        server.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn durable_dead_letters_persist_retry_exhaustion_and_gate_replay() -> Result<()> {
        let stream_server = TestStreamServer::spawn().await?;
        let dead_letter_stream_url =
            stream_server.stream_url(&format!("dead-letter-{}", uuid::Uuid::new_v4()));
        let server = TestWebhookServer::spawn(VecDeque::from([
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
        ]))
        .await?;
        let subscriber = WebhookSubscriber::new(subscriber_config(server.url.clone()));
        let dead_letters = DurableWebhookDeadLetterSink::new(dead_letter_stream_url.clone());

        let result = subscriber
            .dispatch_with_retry(
                prompt_event(),
                &[],
                &MemoryCursorStore::default(),
                Some(&dead_letters),
            )
            .await?;
        let record = match result {
            WebhookDispatchResult::DeadLettered(record) => record,
            other => panic!("expected retry exhaustion to dead-letter, got {other:?}"),
        };
        assert_eq!(record.attempts, 3);
        assert_eq!(
            record
                .meta
                .as_ref()
                .and_then(|meta| meta.traceparent.as_deref()),
            Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01")
        );

        let rows = read_all_json_rows(&dead_letter_stream_url)
            .await?
            .expect("dead-letter stream rows");
        assert_eq!(rows.len(), 1);
        let stored: StateEnvelope<WebhookDeadLetterRecord> =
            serde_json::from_value(rows.into_iter().next().expect("dead-letter row"))?;
        assert_eq!(stored.entity_type, WEBHOOK_DEAD_LETTER_TYPE);
        assert_eq!(
            stored.key,
            dead_letter_storage_key("slack-approvals", &record.completion_key)
        );
        assert_eq!(stored.value, record);

        let skipped = subscriber
            .dispatch_with_retry(
                prompt_event(),
                &[],
                &MemoryCursorStore::default(),
                Some(&DurableWebhookDeadLetterSink::new(dead_letter_stream_url)),
            )
            .await?;
        assert_eq!(
            skipped,
            WebhookDispatchResult::Skipped(WebhookSkipReason::AlreadyDeadLettered)
        );
        assert_eq!(
            server.captured().await.len(),
            3,
            "replay after durable dead-letter persistence must not redeliver the event"
        );

        server.shutdown().await;
        stream_server.shutdown().await;
        Ok(())
    }
}
