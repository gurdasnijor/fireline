use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use fireline_acp_ids::{RequestId, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::{Mutex, Notify};

use crate::durable_subscriber::{
    ActiveSubscriber, CompletionKey, DurableSubscriber, HandlerOutcome, RetryPolicy,
    StreamEnvelope, TraceContext,
};

pub const WAKE_TIMER_REQUESTED_KIND: &str = "wake_timer_requested";
pub const TIMER_FIRED_KIND: &str = "timer_fired";

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

    #[must_use]
    pub fn completion_key(&self) -> CompletionKey {
        CompletionKey::prompt(self.session_id.clone(), self.request_id.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeTimerHandleError {
    AlreadyFired {
        key: CompletionKey,
        fired_at_ms: i64,
    },
    Canceled {
        key: CompletionKey,
    },
}

impl fmt::Display for WakeTimerHandleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyFired { key, fired_at_ms } => write!(
                f,
                "wake timer '{}' already fired at {}",
                key.storage_key(),
                fired_at_ms
            ),
            Self::Canceled { key } => {
                write!(
                    f,
                    "wake timer '{}' was canceled before firing",
                    key.storage_key()
                )
            }
        }
    }
}

impl std::error::Error for WakeTimerHandleError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeTimerCancelError {
    NotScheduled {
        key: CompletionKey,
    },
    AlreadyFired {
        key: CompletionKey,
        fired_at_ms: i64,
    },
}

impl fmt::Display for WakeTimerCancelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotScheduled { key } => {
                write!(f, "wake timer '{}' is not scheduled", key.storage_key())
            }
            Self::AlreadyFired { key, fired_at_ms } => write!(
                f,
                "wake timer '{}' already fired at {}",
                key.storage_key(),
                fired_at_ms
            ),
        }
    }
}

impl std::error::Error for WakeTimerCancelError {}

#[derive(Debug)]
enum TimerEntryState {
    Pending { leader_claimed: bool },
    Fired(TimerFired),
    Canceled,
}

#[derive(Debug)]
struct TimerEntry {
    state: Mutex<TimerEntryState>,
    notify: Notify,
}

impl TimerEntry {
    fn new() -> Self {
        Self {
            state: Mutex::new(TimerEntryState::Pending {
                leader_claimed: false,
            }),
            notify: Notify::new(),
        }
    }
}

/// Active subscriber that waits until a prompt-bound wake deadline then
/// appends a `timer_fired` completion on the same canonical key.
pub struct WakeTimerSubscriber<R = SystemWakeTimerRuntime> {
    name: String,
    runtime: R,
    retry_policy: RetryPolicy,
    entries: Arc<Mutex<HashMap<CompletionKey, Arc<TimerEntry>>>>,
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
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[must_use]
    pub fn with_retry_policy(mut self, retry_policy: RetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }

    pub async fn cancel(
        &self,
        session_id: SessionId,
        request_id: RequestId,
    ) -> Result<(), WakeTimerCancelError> {
        let key = CompletionKey::prompt(session_id, request_id);
        self.cancel_key(&key).await
    }

    pub async fn cancel_key(&self, key: &CompletionKey) -> Result<(), WakeTimerCancelError> {
        let entry = {
            let entries = self.entries.lock().await;
            entries.get(key).cloned()
        }
        .ok_or_else(|| WakeTimerCancelError::NotScheduled { key: key.clone() })?;

        let mut state = entry.state.lock().await;
        match &*state {
            TimerEntryState::Pending { .. } => {
                *state = TimerEntryState::Canceled;
                drop(state);
                entry.notify.notify_waiters();
                Ok(())
            }
            TimerEntryState::Canceled => Ok(()),
            TimerEntryState::Fired(completion) => Err(WakeTimerCancelError::AlreadyFired {
                key: key.clone(),
                fired_at_ms: completion.fired_at_ms,
            }),
        }
    }

    async fn execute(&self, event: WakeTimerRequest) -> Result<TimerFired, WakeTimerHandleError>
    where
        R: WakeTimerRuntime,
    {
        let key = event.completion_key();
        let entry = self.entry_for_key(&key).await;

        if !self.claim_or_wait_for_terminal(&entry, &key).await? {
            return self.terminal_error(&entry, &key).await;
        }

        let remaining = event.remaining_delay(self.runtime.now_ms());
        let canceled = entry.notify.notified();
        tokio::pin!(canceled);

        if !remaining.is_zero() {
            let sleep = self.runtime.sleep(remaining);
            tokio::pin!(sleep);

            tokio::select! {
                _ = &mut sleep => {}
                _ = &mut canceled => {
                    return self.terminal_error(&entry, &key).await;
                }
            }
        }

        let completion = event.completion(self.runtime.now_ms());
        let mut state = entry.state.lock().await;
        match &*state {
            TimerEntryState::Pending { .. } => {
                *state = TimerEntryState::Fired(completion.clone());
                drop(state);
                entry.notify.notify_waiters();
                Ok(completion)
            }
            TimerEntryState::Canceled => Err(WakeTimerHandleError::Canceled { key }),
            TimerEntryState::Fired(existing) => Err(WakeTimerHandleError::AlreadyFired {
                key,
                fired_at_ms: existing.fired_at_ms,
            }),
        }
    }

    async fn entry_for_key(&self, key: &CompletionKey) -> Arc<TimerEntry> {
        let mut entries = self.entries.lock().await;
        entries
            .entry(key.clone())
            .or_insert_with(|| Arc::new(TimerEntry::new()))
            .clone()
    }

    async fn claim_or_wait_for_terminal(
        &self,
        entry: &Arc<TimerEntry>,
        key: &CompletionKey,
    ) -> Result<bool, WakeTimerHandleError> {
        loop {
            let should_wait = {
                let mut state = entry.state.lock().await;
                match &mut *state {
                    TimerEntryState::Pending { leader_claimed } => {
                        if !*leader_claimed {
                            *leader_claimed = true;
                            return Ok(true);
                        }
                        true
                    }
                    TimerEntryState::Fired(existing) => {
                        return Err(WakeTimerHandleError::AlreadyFired {
                            key: key.clone(),
                            fired_at_ms: existing.fired_at_ms,
                        });
                    }
                    TimerEntryState::Canceled => {
                        return Err(WakeTimerHandleError::Canceled { key: key.clone() });
                    }
                }
            };

            if should_wait {
                entry.notify.notified().await;
            }
        }
    }

    async fn terminal_error(
        &self,
        entry: &Arc<TimerEntry>,
        key: &CompletionKey,
    ) -> Result<TimerFired, WakeTimerHandleError> {
        let state = entry.state.lock().await;
        match &*state {
            TimerEntryState::Fired(existing) => Err(WakeTimerHandleError::AlreadyFired {
                key: key.clone(),
                fired_at_ms: existing.fired_at_ms,
            }),
            TimerEntryState::Canceled | TimerEntryState::Pending { .. } => {
                Err(WakeTimerHandleError::Canceled { key: key.clone() })
            }
        }
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
        log.iter().any(|envelope| {
            envelope.kind() == Some(TIMER_FIRED_KIND)
                && envelope.completion_key().as_ref() == Some(&event.completion_key())
        })
    }
}

#[async_trait]
impl<R> ActiveSubscriber for WakeTimerSubscriber<R>
where
    R: WakeTimerRuntime,
{
    async fn handle(&self, event: Self::Event) -> HandlerOutcome<Self::Completion> {
        match self.execute(event).await {
            Ok(completion) => HandlerOutcome::Completed(completion),
            Err(error) => HandlerOutcome::Failed(anyhow!(error)),
        }
    }

    fn retry_policy(&self) -> RetryPolicy {
        self.retry_policy
    }
}

pub fn wake_timer_request_envelope(request: WakeTimerRequest) -> Result<StreamEnvelope> {
    Ok(StreamEnvelope {
        entity_type: "wake_timer".to_string(),
        key: format!(
            "{}:wake_timer_requested",
            request.completion_key().storage_key()
        ),
        headers: insert_headers(),
        source_offset: None,
        value: Some(serde_json::to_value(request)?),
    })
}

pub fn timer_fired_envelope(completion: TimerFired) -> Result<StreamEnvelope> {
    Ok(StreamEnvelope {
        entity_type: "wake_timer".to_string(),
        key: format!("{}:timer_fired", completion.completion_key().storage_key()),
        headers: insert_headers(),
        source_offset: None,
        value: Some(serde_json::to_value(completion)?),
    })
}

fn insert_headers() -> Map<String, Value> {
    Map::from_iter([("operation".to_string(), Value::String("insert".to_string()))])
}
