//! Durable subscriber substrate and profile implementations.
//!
//! Phase 1 landed the trait surface and registration contract. Later phases add
//! concrete profiles without replacing the underlying substrate.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use durable_streams::{Client as DurableStreamsClient, LiveMode, Offset};
use fireline_acp_ids::{PromptRequestRef, RequestId, SessionId, ToolCallId, ToolInvocationRef};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};

pub const WAKE_TIMER_REQUESTED_KIND: &str = "wake_timer_requested";
pub const TIMER_FIRED_KIND: &str = "timer_fired";
pub const DEPLOYMENT_WAKE_REQUESTED_KIND: &str = "deployment_wake_requested";
pub const SANDBOX_PROVISIONED_KIND: &str = "sandbox_provisioned";

/// Canonical completion identity for durable subscribers.
///
/// Prompt/tool completions stay ACP-shaped, and deployment wake profiles reuse
/// canonical `SessionId` rather than introducing a second deployment id.
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
    Session {
        session_id: SessionId,
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
    pub fn session(session_id: SessionId) -> Self {
        Self::Session { session_id }
    }

    #[must_use]
    pub fn session_id(&self) -> &SessionId {
        match self {
            Self::Prompt { session_id, .. }
            | Self::Tool { session_id, .. }
            | Self::Session { session_id } => session_id,
        }
    }

    #[must_use]
    pub fn storage_key(&self) -> String {
        match self {
            Self::Prompt {
                session_id,
                request_id,
            } => format!("prompt:{session_id}:{}", request_id_storage_key(request_id)),
            Self::Tool {
                session_id,
                tool_call_id,
            } => format!("tool:{session_id}:{tool_call_id}"),
            Self::Session { session_id } => format!("session:{session_id}"),
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
            Self::Tool { .. } | Self::Session { .. } => None,
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
            Self::Prompt { .. } | Self::Session { .. } => None,
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

/// Typed view of a stream row consumed by durable subscribers.
///
/// The payload stays JSON-backed but exposes helpers for decoding canonical
/// prompt, tool, and deployment-session identifiers plus trace fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamEnvelope {
    #[serde(rename = "type")]
    pub entity_type: String,
    pub key: String,
    #[serde(default)]
    pub headers: Map<String, Value>,
    /// Reader-local offset metadata used by active subscriber delivery.
    /// This is not an agent-plane field and is not expected on persisted rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_offset: Option<String>,
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
    pub fn kind(&self) -> Option<&str> {
        self.value.as_ref()?.get("kind")?.as_str()
    }

    #[must_use]
    pub fn with_source_offset(mut self, offset: Offset) -> Self {
        self.source_offset = Some(offset.to_string());
        self
    }

    #[must_use]
    pub fn offset(&self) -> Option<Offset> {
        self.source_offset.as_deref().map(Offset::parse)
    }

    #[must_use]
    pub fn without_source_offset(mut self) -> Self {
        self.source_offset = None;
        self
    }

    #[must_use]
    pub fn session_id(&self) -> Option<SessionId> {
        let value = self.value.as_ref()?;
        session_id_from_value(value.get("sessionId")?)
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
            .or_else(|| self.session_id().map(CompletionKey::session))
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

impl RetryPolicy {
    #[must_use]
    pub fn backoff_for_attempt(&self, attempt: u32) -> Option<Duration> {
        if attempt >= self.max_attempts {
            return None;
        }
        if self.initial_backoff.is_zero() {
            return Some(Duration::ZERO);
        }

        let shift = attempt.saturating_sub(1).min(31);
        let multiplier = 1_u32 << shift;
        Some(
            self.initial_backoff
                .saturating_mul(multiplier)
                .min(self.max_backoff),
        )
    }
}

/// Timer runtime abstraction used by the prompt-bound wake timer profile.
#[async_trait]
pub trait WakeTimerRuntime: Send + Sync {
    fn now_ms(&self) -> i64;

    async fn sleep(&self, duration: Duration);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemWakeTimerRuntime;

#[async_trait]
impl WakeTimerRuntime for SystemWakeTimerRuntime {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

/// Agent-bound deferred wake request keyed by canonical prompt identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeTimerRequest {
    pub kind: String,
    pub session_id: SessionId,
    pub request_id: RequestId,
    pub fire_at_ms: i64,
    #[serde(
        default,
        rename = "_meta",
        skip_serializing_if = "TraceContext::is_empty"
    )]
    pub trace_context: TraceContext,
}

impl WakeTimerRequest {
    #[must_use]
    pub fn new(session_id: SessionId, request_id: RequestId, fire_at_ms: i64) -> Self {
        Self {
            kind: WAKE_TIMER_REQUESTED_KIND.to_string(),
            session_id,
            request_id,
            fire_at_ms,
            trace_context: TraceContext::default(),
        }
    }

    #[must_use]
    pub fn with_trace_context(mut self, trace_context: TraceContext) -> Self {
        self.trace_context = trace_context;
        self
    }

    #[must_use]
    pub fn completion_key(&self) -> CompletionKey {
        CompletionKey::prompt(self.session_id.clone(), self.request_id.clone())
    }

    #[must_use]
    pub fn remaining_delay(&self, now_ms: i64) -> Duration {
        let remaining_ms = self.fire_at_ms.saturating_sub(now_ms);
        if remaining_ms <= 0 {
            Duration::ZERO
        } else {
            Duration::from_millis(remaining_ms as u64)
        }
    }

    #[must_use]
    pub fn completion(&self, fired_at_ms: i64) -> TimerFired {
        TimerFired::new(
            self.session_id.clone(),
            self.request_id.clone(),
            fired_at_ms.max(self.fire_at_ms),
        )
        .with_trace_context(self.trace_context.clone())
    }
}

/// Completion appended when a prompt-bound timer fires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerFired {
    pub kind: String,
    pub session_id: SessionId,
    pub request_id: RequestId,
    pub fired_at_ms: i64,
    #[serde(
        default,
        rename = "_meta",
        skip_serializing_if = "TraceContext::is_empty"
    )]
    pub trace_context: TraceContext,
}

impl TimerFired {
    #[must_use]
    pub fn new(session_id: SessionId, request_id: RequestId, fired_at_ms: i64) -> Self {
        Self {
            kind: TIMER_FIRED_KIND.to_string(),
            session_id,
            request_id,
            fired_at_ms,
            trace_context: TraceContext::default(),
        }
    }

    #[must_use]
    pub fn with_trace_context(mut self, trace_context: TraceContext) -> Self {
        self.trace_context = trace_context;
        self
    }
}

/// Active subscriber that waits until a prompt-bound wake deadline then
/// appends a `timer_fired` completion on the same canonical key.
pub struct WakeTimerSubscriber<R = SystemWakeTimerRuntime> {
    name: String,
    runtime: R,
    retry_policy: RetryPolicy,
}

impl WakeTimerSubscriber<SystemWakeTimerRuntime> {
    #[must_use]
    pub fn new() -> Self {
        Self::with_runtime(SystemWakeTimerRuntime)
    }
}

impl Default for WakeTimerSubscriber<SystemWakeTimerRuntime> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R> WakeTimerSubscriber<R> {
    #[must_use]
    pub fn with_runtime(runtime: R) -> Self {
        Self {
            name: "wake_timer".to_string(),
            runtime,
            retry_policy: RetryPolicy::default(),
        }
    }

    #[must_use]
    pub fn with_retry_policy(mut self, retry_policy: RetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }
}

impl<R> DurableSubscriber for WakeTimerSubscriber<R>
where
    R: WakeTimerRuntime,
{
    type Event = WakeTimerRequest;
    type Completion = TimerFired;

    fn name(&self) -> &str {
        &self.name
    }

    fn matches(&self, envelope: &StreamEnvelope) -> Option<Self::Event> {
        let event: WakeTimerRequest = envelope.value_as()?;
        (event.kind == WAKE_TIMER_REQUESTED_KIND).then_some(event)
    }

    fn completion_key(&self, event: &Self::Event) -> CompletionKey {
        event.completion_key()
    }

    fn is_completed(&self, event: &Self::Event, log: &[StreamEnvelope]) -> bool {
        log_contains_completion(log, TIMER_FIRED_KIND, &event.completion_key())
    }
}

#[async_trait]
impl<R> ActiveSubscriber for WakeTimerSubscriber<R>
where
    R: WakeTimerRuntime,
{
    async fn handle(&self, event: Self::Event) -> HandlerOutcome<Self::Completion> {
        let remaining = event.remaining_delay(self.runtime.now_ms());
        if !remaining.is_zero() {
            self.runtime.sleep(remaining).await;
        }
        HandlerOutcome::Completed(event.completion(self.runtime.now_ms()))
    }

    fn retry_policy(&self) -> RetryPolicy {
        self.retry_policy
    }
}

/// Session-scoped request that asks the host to ensure a deployment is awake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentWakeRequested {
    pub kind: String,
    pub session_id: SessionId,
    #[serde(
        default,
        rename = "_meta",
        skip_serializing_if = "TraceContext::is_empty"
    )]
    pub trace_context: TraceContext,
}

impl DeploymentWakeRequested {
    #[must_use]
    pub fn new(session_id: SessionId) -> Self {
        Self {
            kind: DEPLOYMENT_WAKE_REQUESTED_KIND.to_string(),
            session_id,
            trace_context: TraceContext::default(),
        }
    }

    #[must_use]
    pub fn with_trace_context(mut self, trace_context: TraceContext) -> Self {
        self.trace_context = trace_context;
        self
    }

    #[must_use]
    pub fn completion_key(&self) -> CompletionKey {
        CompletionKey::session(self.session_id.clone())
    }
}

/// Completion appended once the existing wake/provision path yields a runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxProvisioned {
    pub kind: String,
    pub session_id: SessionId,
    #[serde(rename = "runtimeKey")]
    pub runtime_key: String,
    #[serde(rename = "runtimeId")]
    pub runtime_id: String,
    #[serde(
        default,
        rename = "_meta",
        skip_serializing_if = "TraceContext::is_empty"
    )]
    pub trace_context: TraceContext,
}

impl SandboxProvisioned {
    #[must_use]
    pub fn new(
        session_id: SessionId,
        runtime_key: impl Into<String>,
        runtime_id: impl Into<String>,
    ) -> Self {
        Self {
            kind: SANDBOX_PROVISIONED_KIND.to_string(),
            session_id,
            runtime_key: runtime_key.into(),
            runtime_id: runtime_id.into(),
            trace_context: TraceContext::default(),
        }
    }

    #[must_use]
    pub fn with_trace_context(mut self, trace_context: TraceContext) -> Self {
        self.trace_context = trace_context;
        self
    }
}

/// Minimal runtime identity surfaced by the wake/provision substrate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionedRuntime {
    pub runtime_key: String,
    pub runtime_id: String,
}

/// Host-owned adapter that translates a deployment wake into the existing
/// session resume/provision path.
#[async_trait]
pub trait DeploymentWakeHandler: Send + Sync {
    async fn wake(&self, session_id: &SessionId) -> Result<ProvisionedRuntime>;
}

#[derive(Clone)]
pub struct ResumeDeploymentWakeHandler {
    http: reqwest::Client,
    control_plane_url: String,
    shared_state_url: String,
}

impl ResumeDeploymentWakeHandler {
    #[must_use]
    pub fn new(
        http: reqwest::Client,
        control_plane_url: impl Into<String>,
        shared_state_url: impl Into<String>,
    ) -> Self {
        Self {
            http,
            control_plane_url: control_plane_url.into(),
            shared_state_url: shared_state_url.into(),
        }
    }
}

#[async_trait]
impl DeploymentWakeHandler for ResumeDeploymentWakeHandler {
    async fn wake(&self, session_id: &SessionId) -> Result<ProvisionedRuntime> {
        let session_id = session_id.to_string();
        let descriptor = fireline_orchestration::resume(
            &self.http,
            &self.control_plane_url,
            &self.shared_state_url,
            &session_id,
        )
        .await?;

        Ok(ProvisionedRuntime {
            runtime_key: descriptor.host_key,
            runtime_id: descriptor.host_id,
        })
    }
}

/// Active subscriber that turns `deployment_wake_requested` into
/// `sandbox_provisioned` by delegating to the existing wake/provision
/// composition helper.
pub struct AlwaysOnDeploymentSubscriber {
    name: String,
    wake_handler: Arc<dyn DeploymentWakeHandler>,
    retry_policy: RetryPolicy,
}

impl AlwaysOnDeploymentSubscriber {
    #[must_use]
    pub fn new(
        http: reqwest::Client,
        control_plane_url: impl Into<String>,
        shared_state_url: impl Into<String>,
    ) -> Self {
        Self::with_wake_handler(ResumeDeploymentWakeHandler::new(
            http,
            control_plane_url,
            shared_state_url,
        ))
    }

    #[must_use]
    pub fn with_wake_handler<H>(wake_handler: H) -> Self
    where
        H: DeploymentWakeHandler + 'static,
    {
        Self {
            name: "always_on_deployment".to_string(),
            wake_handler: Arc::new(wake_handler),
            retry_policy: RetryPolicy::default(),
        }
    }

    #[must_use]
    pub fn with_retry_policy(mut self, retry_policy: RetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }
}

impl DurableSubscriber for AlwaysOnDeploymentSubscriber {
    type Event = DeploymentWakeRequested;
    type Completion = SandboxProvisioned;

    fn name(&self) -> &str {
        &self.name
    }

    fn matches(&self, envelope: &StreamEnvelope) -> Option<Self::Event> {
        let event: DeploymentWakeRequested = envelope.value_as()?;
        (event.kind == DEPLOYMENT_WAKE_REQUESTED_KIND).then_some(event)
    }

    fn completion_key(&self, event: &Self::Event) -> CompletionKey {
        event.completion_key()
    }

    fn is_completed(&self, event: &Self::Event, log: &[StreamEnvelope]) -> bool {
        log_contains_completion(log, SANDBOX_PROVISIONED_KIND, &event.completion_key())
    }
}

#[async_trait]
impl ActiveSubscriber for AlwaysOnDeploymentSubscriber {
    async fn handle(&self, event: Self::Event) -> HandlerOutcome<Self::Completion> {
        let session_id = event.session_id.clone();
        let trace_context = event.trace_context.clone();
        match self.wake_handler.wake(&session_id).await {
            Ok(runtime) => HandlerOutcome::Completed(
                SandboxProvisioned::new(session_id, runtime.runtime_key, runtime.runtime_id)
                    .with_trace_context(trace_context),
            ),
            Err(error) => HandlerOutcome::RetryTransient(error),
        }
    }

    fn retry_policy(&self) -> RetryPolicy {
        self.retry_policy
    }
}

/// Durable subscriber driver.
///
/// The driver now retains passive replay/wait helpers and subscriber
/// registration. Later active profiles hang off the same surface.
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

fn log_contains_completion(log: &[StreamEnvelope], kind: &str, key: &CompletionKey) -> bool {
    log.iter().any(|envelope| {
        envelope.kind() == Some(kind) && envelope.completion_key().as_ref() == Some(key)
    })
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

fn request_id_storage_key(value: &RequestId) -> String {
    match value {
        RequestId::Null => "null".to_string(),
        RequestId::Number(number) => number.to_string(),
        RequestId::Str(text) => text.clone(),
    }
}
#[cfg(test)]
mod tests {
    use std::sync::Mutex;

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

    #[derive(Clone)]
    struct RecordingWakeTimerRuntime {
        now_ms: Arc<Mutex<i64>>,
        sleeps: Arc<Mutex<Vec<Duration>>>,
    }

    impl RecordingWakeTimerRuntime {
        fn new(now_ms: i64) -> Self {
            Self {
                now_ms: Arc::new(Mutex::new(now_ms)),
                sleeps: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn sleeps(&self) -> Vec<Duration> {
            self.sleeps.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl WakeTimerRuntime for RecordingWakeTimerRuntime {
        fn now_ms(&self) -> i64 {
            *self.now_ms.lock().unwrap()
        }

        async fn sleep(&self, duration: Duration) {
            self.sleeps.lock().unwrap().push(duration);
            *self.now_ms.lock().unwrap() += duration.as_millis() as i64;
        }
    }

    #[derive(Clone)]
    struct RecordingDeploymentWakeHandler {
        runtime: ProvisionedRuntime,
        sessions: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingDeploymentWakeHandler {
        fn new(runtime_key: &str, runtime_id: &str) -> Self {
            Self {
                runtime: ProvisionedRuntime {
                    runtime_key: runtime_key.to_string(),
                    runtime_id: runtime_id.to_string(),
                },
                sessions: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn sessions(&self) -> Vec<String> {
            self.sessions.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl DeploymentWakeHandler for RecordingDeploymentWakeHandler {
        async fn wake(&self, session_id: &SessionId) -> Result<ProvisionedRuntime> {
            self.sessions.lock().unwrap().push(session_id.to_string());
            Ok(self.runtime.clone())
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
        assert_eq!(prompt_key.storage_key(), "prompt:session-1:request-1");
        assert_eq!(tool_key.storage_key(), "tool:session-2:tool-1");
    }

    #[test]
    fn completion_key_supports_session_scoped_deployments() {
        let key = CompletionKey::session(SessionId::from("deployment-session"));

        assert_eq!(key.session_id(), &SessionId::from("deployment-session"));
        assert_eq!(key.prompt_ref(), None);
        assert_eq!(key.tool_ref(), None);
        assert_eq!(key.storage_key(), "session:deployment-session");
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

        assert_eq!(envelope.kind(), Some("permission_request"));
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
                "kind": "tool_invoked",
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
    fn stream_envelope_extracts_session_key_when_prompt_and_tool_keys_are_absent() {
        let envelope = StreamEnvelope::from_json(serde_json::json!({
            "type": "deployment",
            "key": "deployment-session",
            "headers": {},
            "value": {
                "kind": DEPLOYMENT_WAKE_REQUESTED_KIND,
                "sessionId": "deployment-session"
            }
        }))
        .expect("decode stream envelope");

        assert_eq!(
            envelope.completion_key(),
            Some(CompletionKey::session(SessionId::from(
                "deployment-session"
            )))
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
        assert_eq!(RetryPolicy::default().backoff_for_attempt(1), None);
    }

    #[test]
    fn stream_envelope_tracks_reader_offset_out_of_band() {
        let envelope = StreamEnvelope::from_json(serde_json::json!({
            "type": "permission",
            "key": "session-1:request-1",
            "headers": {},
            "value": {
                "kind": "permission_request",
                "sessionId": "session-1",
                "requestId": "request-1"
            }
        }))
        .expect("decode stream envelope")
        .with_source_offset(Offset::at("0000000000000001_0000000000000002"));

        assert_eq!(
            envelope.offset(),
            Some(Offset::at("0000000000000001_0000000000000002"))
        );
        assert_eq!(envelope.clone().without_source_offset().source_offset, None);
    }

    #[test]
    fn retry_policy_backoff_caps_at_maximum() {
        let policy = RetryPolicy {
            max_attempts: 5,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_millis(250),
        };

        assert_eq!(
            policy.backoff_for_attempt(1),
            Some(Duration::from_millis(100))
        );
        assert_eq!(
            policy.backoff_for_attempt(2),
            Some(Duration::from_millis(200))
        );
        assert_eq!(
            policy.backoff_for_attempt(3),
            Some(Duration::from_millis(250))
        );
        assert_eq!(
            policy.backoff_for_attempt(4),
            Some(Duration::from_millis(250))
        );
        assert_eq!(policy.backoff_for_attempt(5), None);
    }

    #[tokio::test]
    async fn wake_timer_subscriber_replay_restores_pending_wait_using_remaining_delay() {
        let runtime = RecordingWakeTimerRuntime::new(1_250);
        let subscriber = WakeTimerSubscriber::with_runtime(runtime.clone());
        let event = WakeTimerRequest::new(
            SessionId::from("session-1"),
            RequestId::from("request-1".to_string()),
            1_600,
        )
        .with_trace_context(TraceContext {
            traceparent: Some("00-trace-01".to_string()),
            tracestate: None,
            baggage: None,
        });

        let outcome = subscriber.handle(event).await;
        let completion = match outcome {
            HandlerOutcome::Completed(completion) => completion,
            HandlerOutcome::RetryTransient(error) | HandlerOutcome::Failed(error) => {
                panic!("unexpected wake timer failure: {error:#}")
            }
        };

        assert_eq!(runtime.sleeps(), vec![Duration::from_millis(350)]);
        assert_eq!(
            completion,
            TimerFired::new(
                SessionId::from("session-1"),
                RequestId::from("request-1".to_string()),
                1_600
            )
            .with_trace_context(TraceContext {
                traceparent: Some("00-trace-01".to_string()),
                tracestate: None,
                baggage: None,
            })
        );
    }

    #[test]
    fn wake_timer_subscriber_detects_existing_completion() {
        let subscriber = WakeTimerSubscriber::new();
        let event = WakeTimerRequest::new(
            SessionId::from("session-1"),
            RequestId::from("request-1".to_string()),
            1_600,
        );
        let log = vec![
            StreamEnvelope::from_json(serde_json::json!({
                "type": "completion",
                "key": "session-1:request-1:timer",
                "headers": {},
                "value": {
                    "kind": TIMER_FIRED_KIND,
                    "sessionId": "session-1",
                    "requestId": "request-1",
                    "firedAtMs": 1600
                }
            }))
            .expect("decode timer completion envelope"),
        ];

        assert!(subscriber.is_completed(&event, &log));
    }

    #[tokio::test]
    async fn always_on_deployment_subscriber_delegates_to_wake_handler() {
        let wake_handler = RecordingDeploymentWakeHandler::new("runtime-key-1", "runtime-id-1");
        let subscriber = AlwaysOnDeploymentSubscriber::with_wake_handler(wake_handler.clone())
            .with_retry_policy(RetryPolicy {
                max_attempts: 3,
                initial_backoff: Duration::from_millis(50),
                max_backoff: Duration::from_secs(1),
            });
        let event = DeploymentWakeRequested::new(SessionId::from("deployment-session"))
            .with_trace_context(TraceContext {
                traceparent: Some("00-deploy-trace".to_string()),
                tracestate: Some("vendor=value".to_string()),
                baggage: None,
            });

        let outcome = subscriber.handle(event).await;
        let completion = match outcome {
            HandlerOutcome::Completed(completion) => completion,
            HandlerOutcome::RetryTransient(error) | HandlerOutcome::Failed(error) => {
                panic!("unexpected deployment wake failure: {error:#}")
            }
        };

        assert_eq!(
            wake_handler.sessions(),
            vec!["deployment-session".to_string()]
        );
        assert_eq!(
            completion,
            SandboxProvisioned::new(
                SessionId::from("deployment-session"),
                "runtime-key-1",
                "runtime-id-1",
            )
            .with_trace_context(TraceContext {
                traceparent: Some("00-deploy-trace".to_string()),
                tracestate: Some("vendor=value".to_string()),
                baggage: None,
            })
        );
    }

    #[test]
    fn always_on_deployment_subscriber_detects_existing_provisioning_completion() {
        let subscriber = AlwaysOnDeploymentSubscriber::with_wake_handler(
            RecordingDeploymentWakeHandler::new("runtime-key-1", "runtime-id-1"),
        );
        let event = DeploymentWakeRequested::new(SessionId::from("deployment-session"));
        let log = vec![
            StreamEnvelope::from_json(serde_json::json!({
                "type": "deployment",
                "key": "deployment-session:ready",
                "headers": {},
                "value": {
                    "kind": SANDBOX_PROVISIONED_KIND,
                    "sessionId": "deployment-session",
                    "runtimeKey": "runtime-key-1",
                    "runtimeId": "runtime-id-1"
                }
            }))
            .expect("decode sandbox_provisioned envelope"),
        ];

        assert!(subscriber.is_completed(&event, &log));
    }
}
