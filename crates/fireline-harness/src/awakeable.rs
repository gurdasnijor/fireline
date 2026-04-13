use std::future::Future;
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use fireline_acp_ids::{RequestId, SessionId, ToolCallId};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};

use crate::durable_subscriber::{
    CompletionKey, DurableSubscriber, DurableSubscriberDriver, PassiveSubscriber, StreamEnvelope,
    TraceContext,
};

pub const AWAKEABLE_WAITING_KIND: &str = "awakeable_waiting";
pub const AWAKEABLE_RESOLVED_KIND: &str = "awakeable_resolved";

/// Awakeables reuse the canonical durable-subscriber completion key surface.
/// This is an alias, not a second semantic identifier type.
pub type AwakeableKey = CompletionKey;

/// Agent-plane wait declaration used by the passive awakeable subscriber.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AwakeableWaiting {
    pub kind: String,
    pub session_id: SessionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<RequestId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<ToolCallId>,
    pub created_at_ms: i64,
    #[serde(
        default,
        rename = "_meta",
        skip_serializing_if = "TraceContext::is_empty"
    )]
    pub trace_context: TraceContext,
}

impl AwakeableWaiting {
    #[must_use]
    pub fn new(key: AwakeableKey) -> Self {
        let parts = AwakeableKeyParts::from_key(&key);
        Self {
            kind: AWAKEABLE_WAITING_KIND.to_string(),
            session_id: parts.session_id,
            request_id: parts.request_id,
            tool_call_id: parts.tool_call_id,
            created_at_ms: now_ms(),
            trace_context: TraceContext::default(),
        }
    }

    #[must_use]
    pub fn with_trace_context(mut self, trace_context: TraceContext) -> Self {
        self.trace_context = trace_context;
        self
    }

    #[must_use]
    pub fn completion_key(&self) -> AwakeableKey {
        completion_key_from_parts(
            self.session_id.clone(),
            self.request_id.clone(),
            self.tool_call_id.clone(),
        )
    }
}

/// Generic awakeable completion payload carried on the agent plane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AwakeableResolved<T> {
    pub kind: String,
    pub session_id: SessionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<RequestId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<ToolCallId>,
    pub value: T,
    pub resolved_at_ms: i64,
    #[serde(
        default,
        rename = "_meta",
        skip_serializing_if = "TraceContext::is_empty"
    )]
    pub trace_context: TraceContext,
}

impl<T> AwakeableResolved<T> {
    #[must_use]
    pub fn new(key: AwakeableKey, value: T) -> Self {
        let parts = AwakeableKeyParts::from_key(&key);
        Self {
            kind: AWAKEABLE_RESOLVED_KIND.to_string(),
            session_id: parts.session_id,
            request_id: parts.request_id,
            tool_call_id: parts.tool_call_id,
            value,
            resolved_at_ms: now_ms(),
            trace_context: TraceContext::default(),
        }
    }

    #[must_use]
    pub fn with_trace_context(mut self, trace_context: TraceContext) -> Self {
        self.trace_context = trace_context;
        self
    }

    #[must_use]
    pub fn completion_key(&self) -> AwakeableKey {
        completion_key_from_parts(
            self.session_id.clone(),
            self.request_id.clone(),
            self.tool_call_id.clone(),
        )
    }
}

/// Passive durable-subscriber profile backing the imperative awakeable surface.
#[derive(Debug, Clone, Copy, Default)]
pub struct AwakeableSubscriber;

impl AwakeableSubscriber {
    pub const NAME: &str = "awakeable";

    #[must_use]
    pub fn new() -> Self {
        Self
    }

    pub fn completion_for_key<T>(key: &AwakeableKey, log: &[StreamEnvelope]) -> Result<T>
    where
        T: DeserializeOwned,
    {
        log.iter()
            .find_map(|envelope| {
                (envelope.kind() == Some(AWAKEABLE_RESOLVED_KIND)
                    && envelope.completion_key().as_ref() == Some(key))
                .then_some(envelope)
            })
            .ok_or_else(|| anyhow!("awakeable completion missing for key {}", key.storage_key()))
            .and_then(|envelope| {
                envelope
                    .value_as::<AwakeableResolved<T>>()
                    .map(|resolved| resolved.value)
                    .ok_or_else(|| {
                        anyhow!(
                            "decode awakeable_resolved payload for key {}",
                            key.storage_key()
                        )
                    })
            })
    }
}

impl DurableSubscriber for AwakeableSubscriber {
    type Event = AwakeableWaiting;
    type Completion = Value;

    fn name(&self) -> &str {
        Self::NAME
    }

    fn matches(&self, envelope: &StreamEnvelope) -> Option<Self::Event> {
        let event: AwakeableWaiting = envelope.value_as()?;
        (event.kind == AWAKEABLE_WAITING_KIND).then_some(event)
    }

    fn completion_key(&self, event: &Self::Event) -> AwakeableKey {
        event.completion_key()
    }

    fn is_completed(&self, event: &Self::Event, log: &[StreamEnvelope]) -> bool {
        log.iter().any(|envelope| {
            envelope.kind() == Some(AWAKEABLE_RESOLVED_KIND)
                && envelope.completion_key().as_ref() == Some(&event.completion_key())
        })
    }
}

impl PassiveSubscriber for AwakeableSubscriber {}

#[must_use]
pub struct AwakeableFuture<T> {
    key: AwakeableKey,
    inner: Pin<Box<dyn Future<Output = Result<T>> + Send + 'static>>,
}

impl<T> AwakeableFuture<T>
where
    T: DeserializeOwned + Send + 'static,
{
    pub(crate) fn new(
        state_stream_url: impl Into<String>,
        subscriber_driver: std::sync::Arc<DurableSubscriberDriver>,
        key: AwakeableKey,
    ) -> Self {
        let state_stream_url = state_stream_url.into();
        let key_for_wait = key.clone();
        let inner = match awakeable_waiting_envelope(key.clone()) {
            Ok(wait_event) => Box::pin(async move {
                let log = subscriber_driver
                    .wait_for_passive_completion(
                        AwakeableSubscriber::NAME,
                        &wait_event,
                        &state_stream_url,
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "wait for awakeable completion on '{}'",
                            key_for_wait.storage_key()
                        )
                    })?;
                AwakeableSubscriber::completion_for_key::<T>(&key_for_wait, &log)
            })
                as Pin<Box<dyn Future<Output = Result<T>> + Send + 'static>>,
            Err(error) => Box::pin(async move { Err(error) }),
        };

        Self { key, inner }
    }

    #[must_use]
    pub fn key(&self) -> &AwakeableKey {
        &self.key
    }
}

impl<T> Future for AwakeableFuture<T>
where
    T: DeserializeOwned + Send + 'static,
{
    type Output = Result<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        self.inner.as_mut().poll(cx)
    }
}

pub fn awakeable_waiting_envelope(key: AwakeableKey) -> Result<StreamEnvelope> {
    Ok(StreamEnvelope {
        entity_type: "awakeable".to_string(),
        key: format!("{}:waiting", key.storage_key()),
        headers: insert_headers(),
        source_offset: None,
        value: Some(serde_json::to_value(AwakeableWaiting::new(key))?),
    })
}

pub fn awakeable_resolution_envelope<T>(key: AwakeableKey, value: T) -> Result<StreamEnvelope>
where
    T: Serialize,
{
    Ok(StreamEnvelope {
        entity_type: "awakeable".to_string(),
        key: format!("{}:resolved", key.storage_key()),
        headers: insert_headers(),
        source_offset: None,
        value: Some(serde_json::to_value(AwakeableResolved::new(key, value))?),
    })
}

#[derive(Debug, Clone)]
struct AwakeableKeyParts {
    session_id: SessionId,
    request_id: Option<RequestId>,
    tool_call_id: Option<ToolCallId>,
}

impl AwakeableKeyParts {
    fn from_key(key: &AwakeableKey) -> Self {
        match key {
            AwakeableKey::Prompt {
                session_id,
                request_id,
            } => Self {
                session_id: session_id.clone(),
                request_id: Some(request_id.clone()),
                tool_call_id: None,
            },
            AwakeableKey::Tool {
                session_id,
                tool_call_id,
            } => Self {
                session_id: session_id.clone(),
                request_id: None,
                tool_call_id: Some(tool_call_id.clone()),
            },
            AwakeableKey::Session { session_id } => Self {
                session_id: session_id.clone(),
                request_id: None,
                tool_call_id: None,
            },
        }
    }
}

fn completion_key_from_parts(
    session_id: SessionId,
    request_id: Option<RequestId>,
    tool_call_id: Option<ToolCallId>,
) -> AwakeableKey {
    match (request_id, tool_call_id) {
        (Some(request_id), None) => AwakeableKey::prompt(session_id, request_id),
        (None, Some(tool_call_id)) => AwakeableKey::tool(session_id, tool_call_id),
        (None, None) => AwakeableKey::session(session_id),
        (Some(_), Some(_)) => unreachable!("awakeable key cannot be both prompt and tool scoped"),
    }
}

fn insert_headers() -> Map<String, Value> {
    Map::from_iter([("operation".to_string(), Value::String("insert".to_string()))])
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
