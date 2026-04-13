//! Durable subscriber substrate scaffolding.
//!
//! Phase 1 intentionally lands only the Rust trait and registration surface.
//! The driver remains inert until Phase 2 ports the approval gate onto it.

use std::fmt;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use durable_streams::{Client as DurableStreamsClient, LiveMode, Offset};
use fireline_acp_ids::{PromptRequestRef, RequestId, SessionId, ToolCallId, ToolInvocationRef};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{Map, Value};

/// Canonical completion identity for agent-bound durable subscribers.
///
/// The first cut intentionally admits only ACP-shaped prompt and tool
/// references. Infrastructure-only identifiers stay outside this surface.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompletionKey {
    Prompt {
        session_id: SessionId,
        request_id: RequestId,
    },
    Tool {
        session_id: SessionId,
        tool_call_id: ToolCallId,
    },
}

impl CompletionKey {
    #[must_use]
    pub fn prompt(session_id: SessionId, request_id: RequestId) -> Self {
        Self::Prompt {
            session_id,
            request_id,
        }
    }

    #[must_use]
    pub fn tool(session_id: SessionId, tool_call_id: ToolCallId) -> Self {
        Self::Tool {
            session_id,
            tool_call_id,
        }
    }

    #[must_use]
    pub fn session_id(&self) -> &SessionId {
        match self {
            Self::Prompt { session_id, .. } | Self::Tool { session_id, .. } => session_id,
        }
    }

    #[must_use]
    pub fn prompt_ref(&self) -> Option<PromptRequestRef> {
        match self {
            Self::Prompt {
                session_id,
                request_id,
            } => Some(PromptRequestRef {
                session_id: session_id.clone(),
                request_id: request_id.clone(),
            }),
            Self::Tool { .. } => None,
        }
    }

    #[must_use]
    pub fn tool_ref(&self) -> Option<ToolInvocationRef> {
        match self {
            Self::Tool {
                session_id,
                tool_call_id,
            } => Some(ToolInvocationRef {
                session_id: session_id.clone(),
                tool_call_id: tool_call_id.clone(),
            }),
            Self::Prompt { .. } => None,
        }
    }
}

impl From<PromptRequestRef> for CompletionKey {
    fn from(value: PromptRequestRef) -> Self {
        Self::prompt(value.session_id, value.request_id)
    }
}

impl From<ToolInvocationRef> for CompletionKey {
    fn from(value: ToolInvocationRef) -> Self {
        Self::tool(value.session_id, value.tool_call_id)
    }
}

/// W3C trace context copied through subscriber-side effects and completions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracestate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baggage: Option<String>,
}

impl TraceContext {
    #[must_use]
    pub fn from_meta(meta: &Map<String, Value>) -> Self {
        Self {
            traceparent: non_empty_meta_value(meta, "traceparent"),
            tracestate: non_empty_meta_value(meta, "tracestate"),
            baggage: non_empty_meta_value(meta, "baggage"),
        }
    }

    #[must_use]
    pub fn into_meta(self) -> Map<String, Value> {
        let mut meta = Map::new();
        if let Some(traceparent) = self.traceparent {
            meta.insert("traceparent".to_string(), Value::String(traceparent));
        }
        if let Some(tracestate) = self.tracestate {
            meta.insert("tracestate".to_string(), Value::String(tracestate));
        }
        if let Some(baggage) = self.baggage {
            meta.insert("baggage".to_string(), Value::String(baggage));
        }
        meta
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.traceparent.is_none() && self.tracestate.is_none() && self.baggage.is_none()
    }
}

/// Typed view of an agent-plane stream row.
///
/// Phase 1 keeps the payload as JSON but exposes helpers for decoding the
/// canonical ACP identifier and trace fields future subscribers consume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamEnvelope {
    #[serde(rename = "type")]
    pub entity_type: String,
    pub key: String,
    #[serde(default)]
    pub headers: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

impl StreamEnvelope {
    pub fn from_json(value: Value) -> serde_json::Result<Self> {
        serde_json::from_value(value)
    }

    #[must_use]
    pub fn value_as<T>(&self) -> Option<T>
    where
        T: DeserializeOwned,
    {
        self.value
            .clone()
            .and_then(|value| serde_json::from_value(value).ok())
    }

    #[must_use]
    pub fn prompt_ref(&self) -> Option<PromptRequestRef> {
        let value = self.value.as_ref()?;
        Some(PromptRequestRef {
            session_id: session_id_from_value(value.get("sessionId")?)?,
            request_id: request_id_from_value(value.get("requestId")?)?,
        })
    }

    #[must_use]
    pub fn tool_ref(&self) -> Option<ToolInvocationRef> {
        let value = self.value.as_ref()?;
        Some(ToolInvocationRef {
            session_id: session_id_from_value(value.get("sessionId")?)?,
            tool_call_id: tool_call_id_from_value(value.get("toolCallId")?)?,
        })
    }

    #[must_use]
    pub fn completion_key(&self) -> Option<CompletionKey> {
        self.prompt_ref()
            .map(CompletionKey::from)
            .or_else(|| self.tool_ref().map(CompletionKey::from))
    }

    #[must_use]
    pub fn trace_context(&self) -> Option<TraceContext> {
        let meta = self
            .value
            .as_ref()?
            .get("_meta")?
            .as_object()
            .map(TraceContext::from_meta)?;
        (!meta.is_empty()).then_some(meta)
    }
}

/// Registration mode for the inert Phase 1 driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriberMode {
    Passive,
    Active,
}

/// Snapshot of a registered durable subscriber.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriberRegistration {
    pub name: String,
    pub mode: SubscriberMode,
}

impl SubscriberRegistration {
    fn new(name: impl Into<String>, mode: SubscriberMode) -> Self {
        Self {
            name: name.into(),
            mode,
        }
    }
}

/// Shared contract for durable subscribers.
///
/// Implementations must derive completion identity only from canonical ACP
/// identifiers already present in the matched event.
pub trait DurableSubscriber: Send + Sync {
    type Event: DeserializeOwned + Send + Sync + 'static;
    type Completion: Serialize + Send + Sync + 'static;

    /// Infrastructure-facing name used for metrics, config lookup, and admin
    /// UX. This is not an agent-plane identifier.
    fn name(&self) -> &str;

    /// Match and decode a typed event from an agent-plane stream envelope.
    fn matches(&self, envelope: &StreamEnvelope) -> Option<Self::Event>;

    /// Canonical completion identity derived from fields already present on the
    /// matched event.
    fn completion_key(&self, event: &Self::Event) -> CompletionKey;

    /// Whether the provided log already contains a matching completion.
    fn is_completed(&self, event: &Self::Event, log: &[StreamEnvelope]) -> bool;
}

/// Passive subscribers suspend until some other writer appends the completion.
pub trait PassiveSubscriber: DurableSubscriber {
    fn wait_policy(&self) -> PassiveWaitPolicy {
        PassiveWaitPolicy::default()
    }
}

/// Active subscribers perform the side effect that eventually yields the
/// completion envelope.
#[async_trait]
pub trait ActiveSubscriber: DurableSubscriber {
    async fn handle(&self, event: Self::Event) -> HandlerOutcome<Self::Completion>;

    fn retry_policy(&self) -> RetryPolicy {
        RetryPolicy::default()
    }
}

/// Terminal and retryable outcomes for active subscriber dispatch.
#[derive(Debug)]
pub enum HandlerOutcome<C> {
    Completed(C),
    RetryTransient(anyhow::Error),
    Failed(anyhow::Error),
}

/// Passive wait knobs. Phase 2 ports approval timeout behavior onto this.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PassiveWaitPolicy {
    pub timeout: Option<Duration>,
}

/// Bounded retry configuration for active subscriber delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total attempts, including the first dispatch attempt.
    pub max_attempts: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            initial_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
        }
    }
}

/// Phase 1 registration-only driver.
///
/// The driver records the durable subscriber inventory without dispatching or
/// replaying anything yet. That keeps the substrate inert until the first real
/// consumer ports onto it in Phase 2.
#[derive(Default)]
pub struct DurableSubscriberDriver {
    registrations: Vec<Box<dyn ErasedRegistration>>,
}

impl DurableSubscriberDriver {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_passive<S>(&mut self, subscriber: S) -> &mut Self
    where
        S: PassiveSubscriber + 'static,
    {
        self.registrations
            .push(Box::new(PassiveRegistration { subscriber }));
        self
    }

    pub fn register_active<S>(&mut self, subscriber: S) -> &mut Self
    where
        S: ActiveSubscriber + 'static,
    {
        self.registrations
            .push(Box::new(ActiveRegistration { subscriber }));
        self
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.registrations.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.registrations.len()
    }

    #[must_use]
    pub fn registrations(&self) -> Vec<SubscriberRegistration> {
        self.registrations
            .iter()
            .map(|entry| entry.snapshot())
            .collect()
    }

    pub async fn replay_log(&self, state_stream_url: &str) -> Result<Vec<StreamEnvelope>> {
        collect_stream_log(state_stream_url, LiveMode::Off, None, |_| false).await
    }

    pub async fn wait_for_passive_completion(
        &self,
        subscriber_name: &str,
        event: &StreamEnvelope,
        state_stream_url: &str,
    ) -> Result<Vec<StreamEnvelope>> {
        let registration = self
            .find_passive_registration(subscriber_name)
            .ok_or_else(|| {
                anyhow!("no passive durable subscriber registered as '{subscriber_name}'")
            })?;

        if !registration.passive_matches(event) {
            return Err(anyhow!(
                "subscriber '{subscriber_name}' does not match the provided event envelope"
            ));
        }

        let timeout = registration
            .passive_wait_policy()
            .map(|policy| policy.timeout)
            .unwrap_or_default();

        collect_stream_log(state_stream_url, LiveMode::Sse, timeout, |log| {
            registration.passive_is_completed(event, log)
        })
        .await
    }

    fn find_passive_registration(&self, subscriber_name: &str) -> Option<&dyn ErasedRegistration> {
        self.registrations.iter().find_map(|entry| {
            (entry.snapshot().name == subscriber_name && entry.passive_wait_policy().is_some())
                .then_some(entry.as_ref())
        })
    }
}

impl fmt::Debug for DurableSubscriberDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DurableSubscriberDriver")
            .field("registrations", &self.registrations())
            .finish()
    }
}

trait ErasedRegistration: Send + Sync {
    fn snapshot(&self) -> SubscriberRegistration;

    fn passive_matches(&self, _envelope: &StreamEnvelope) -> bool {
        false
    }

    fn passive_is_completed(&self, _event: &StreamEnvelope, _log: &[StreamEnvelope]) -> bool {
        false
    }

    fn passive_wait_policy(&self) -> Option<PassiveWaitPolicy> {
        None
    }
}

struct PassiveRegistration<S> {
    subscriber: S,
}

impl<S> ErasedRegistration for PassiveRegistration<S>
where
    S: PassiveSubscriber,
{
    fn snapshot(&self) -> SubscriberRegistration {
        SubscriberRegistration::new(self.subscriber.name(), SubscriberMode::Passive)
    }

    fn passive_matches(&self, envelope: &StreamEnvelope) -> bool {
        self.subscriber.matches(envelope).is_some()
    }

    fn passive_is_completed(&self, event: &StreamEnvelope, log: &[StreamEnvelope]) -> bool {
        self.subscriber
            .matches(event)
            .map(|decoded| self.subscriber.is_completed(&decoded, log))
            .unwrap_or(false)
    }

    fn passive_wait_policy(&self) -> Option<PassiveWaitPolicy> {
        Some(self.subscriber.wait_policy())
    }
}

struct ActiveRegistration<S> {
    subscriber: S,
}

impl<S> ErasedRegistration for ActiveRegistration<S>
where
    S: ActiveSubscriber,
{
    fn snapshot(&self) -> SubscriberRegistration {
        SubscriberRegistration::new(self.subscriber.name(), SubscriberMode::Active)
    }
}

fn non_empty_meta_value(meta: &Map<String, Value>, field: &str) -> Option<String> {
    meta.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn session_id_from_value(value: &Value) -> Option<SessionId> {
    value.as_str().map(|text| SessionId::from(text.to_string()))
}

fn request_id_from_value(value: &Value) -> Option<RequestId> {
    serde_json::from_value(value.clone()).ok()
}

fn tool_call_id_from_value(value: &Value) -> Option<ToolCallId> {
    value
        .as_str()
        .map(|text| ToolCallId::from(text.to_string()))
}

async fn collect_stream_log(
    state_stream_url: &str,
    live_mode: LiveMode,
    timeout: Option<Duration>,
    mut done: impl FnMut(&[StreamEnvelope]) -> bool,
) -> Result<Vec<StreamEnvelope>> {
    let client = DurableStreamsClient::new();
    let stream = client.stream(state_stream_url);
    let is_replay_only = live_mode == LiveMode::Off;
    let mut reader = stream
        .read()
        .offset(Offset::Beginning)
        .live(live_mode)
        .build()
        .with_context(|| format!("build durable subscriber reader for '{state_stream_url}'"))?;

    let deadline = timeout.map(|duration| tokio::time::Instant::now() + duration);
    let mut log = Vec::new();

    loop {
        let next_chunk = if let Some(deadline) = deadline {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(anyhow!(
                    "timed out waiting for passive durable subscriber completion on '{state_stream_url}'"
                ));
            }
            match tokio::time::timeout(remaining, reader.next_chunk()).await {
                Ok(result) => result,
                Err(_) => {
                    return Err(anyhow!(
                        "timed out waiting for passive durable subscriber completion on '{state_stream_url}'"
                    ));
                }
            }
        } else {
            reader.next_chunk().await
        }
        .with_context(|| format!("read durable subscriber stream '{state_stream_url}'"))?;

        let Some(chunk) = next_chunk else {
            return if is_replay_only {
                Ok(log)
            } else {
                Err(anyhow!(
                    "durable subscriber stream closed before completion on '{state_stream_url}'"
                ))
            };
        };

        if !chunk.data.is_empty() {
            log.extend(parse_chunk_envelopes(&chunk.data)?);
            if done(&log) {
                return Ok(log);
            }
        }

        if is_replay_only && chunk.up_to_date {
            return Ok(log);
        }
    }
}

fn parse_chunk_envelopes(bytes: &[u8]) -> Result<Vec<StreamEnvelope>> {
    let events: Vec<Value> =
        serde_json::from_slice(bytes).context("parse durable subscriber stream chunk as JSON")?;
    events
        .into_iter()
        .map(|event| StreamEnvelope::from_json(event).context("decode durable subscriber envelope"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct DummyPromptEvent {
        session_id: SessionId,
        request_id: RequestId,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
    struct DummyCompletion {
        ok: bool,
    }

    struct DummyPassiveSubscriber;

    impl DurableSubscriber for DummyPassiveSubscriber {
        type Event = DummyPromptEvent;
        type Completion = DummyCompletion;

        fn name(&self) -> &str {
            "dummy_passive"
        }

        fn matches(&self, envelope: &StreamEnvelope) -> Option<Self::Event> {
            envelope.value_as()
        }

        fn completion_key(&self, event: &Self::Event) -> CompletionKey {
            CompletionKey::prompt(event.session_id.clone(), event.request_id.clone())
        }

        fn is_completed(&self, _event: &Self::Event, _log: &[StreamEnvelope]) -> bool {
            false
        }
    }

    impl PassiveSubscriber for DummyPassiveSubscriber {}

    struct DummyActiveSubscriber;

    impl DurableSubscriber for DummyActiveSubscriber {
        type Event = DummyPromptEvent;
        type Completion = DummyCompletion;

        fn name(&self) -> &str {
            "dummy_active"
        }

        fn matches(&self, envelope: &StreamEnvelope) -> Option<Self::Event> {
            envelope.value_as()
        }

        fn completion_key(&self, event: &Self::Event) -> CompletionKey {
            CompletionKey::prompt(event.session_id.clone(), event.request_id.clone())
        }

        fn is_completed(&self, _event: &Self::Event, _log: &[StreamEnvelope]) -> bool {
            false
        }
    }

    #[async_trait]
    impl ActiveSubscriber for DummyActiveSubscriber {
        async fn handle(&self, _event: Self::Event) -> HandlerOutcome<Self::Completion> {
            HandlerOutcome::Completed(DummyCompletion { ok: true })
        }
    }

    #[test]
    fn completion_key_round_trips_canonical_refs() {
        let prompt_key = CompletionKey::from(PromptRequestRef {
            session_id: SessionId::from("session-1"),
            request_id: RequestId::from("request-1".to_string()),
        });
        let tool_key = CompletionKey::from(ToolInvocationRef {
            session_id: SessionId::from("session-2"),
            tool_call_id: ToolCallId::from("tool-1".to_string()),
        });

        assert_eq!(
            prompt_key.prompt_ref(),
            Some(PromptRequestRef {
                session_id: SessionId::from("session-1"),
                request_id: RequestId::from("request-1".to_string()),
            })
        );
        assert_eq!(
            tool_key.tool_ref(),
            Some(ToolInvocationRef {
                session_id: SessionId::from("session-2"),
                tool_call_id: ToolCallId::from("tool-1".to_string()),
            })
        );
    }

    #[test]
    fn stream_envelope_extracts_prompt_key_and_trace_context() {
        let envelope = StreamEnvelope::from_json(serde_json::json!({
            "type": "permission",
            "key": "session-1:request-1",
            "headers": { "operation": "insert" },
            "value": {
                "kind": "permission_request",
                "sessionId": "session-1",
                "requestId": "request-1",
                "_meta": {
                    "traceparent": "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01",
                    "tracestate": "vendor=value"
                }
            }
        }))
        .expect("decode stream envelope");

        assert_eq!(
            envelope.completion_key(),
            Some(CompletionKey::prompt(
                SessionId::from("session-1"),
                RequestId::from("request-1".to_string())
            ))
        );
        assert_eq!(
            envelope.trace_context(),
            Some(TraceContext {
                traceparent: Some(
                    "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string()
                ),
                tracestate: Some("vendor=value".to_string()),
                baggage: None,
            })
        );
    }

    #[test]
    fn stream_envelope_extracts_tool_key_when_prompt_key_is_absent() {
        let envelope = StreamEnvelope::from_json(serde_json::json!({
            "type": "tool_call",
            "key": "session-2:tool-9",
            "headers": {},
            "value": {
                "sessionId": "session-2",
                "toolCallId": "tool-9"
            }
        }))
        .expect("decode stream envelope");

        assert_eq!(
            envelope.completion_key(),
            Some(CompletionKey::tool(
                SessionId::from("session-2"),
                ToolCallId::from("tool-9".to_string())
            ))
        );
    }

    #[test]
    fn driver_registers_active_and_passive_subscribers_without_dispatch() {
        let mut driver = DurableSubscriberDriver::new();
        driver
            .register_passive(DummyPassiveSubscriber)
            .register_active(DummyActiveSubscriber);

        assert_eq!(driver.len(), 2);
        assert_eq!(
            driver.registrations(),
            vec![
                SubscriberRegistration {
                    name: "dummy_passive".to_string(),
                    mode: SubscriberMode::Passive,
                },
                SubscriberRegistration {
                    name: "dummy_active".to_string(),
                    mode: SubscriberMode::Active,
                },
            ]
        );
    }

    #[test]
    fn retry_policy_defaults_to_one_attempt_without_backoff() {
        assert_eq!(
            RetryPolicy::default(),
            RetryPolicy {
                max_attempts: 1,
                initial_backoff: Duration::ZERO,
                max_backoff: Duration::ZERO,
            }
        );
    }
}
