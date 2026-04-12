//! Shared runtime-side durable state materializer.
//!
//! One runtime should keep one durable-streams subscriber / replay loop and
//! fan out decoded `STATE-PROTOCOL` events to narrow in-memory projections.
//! The projections remain replaceable and in-memory only; the durable state
//! stream remains the sole durable source of truth.
//!
//! This implementation follows the Durable Streams State Protocol v1.0
//! (`packages/state/STATE-PROTOCOL.md` upstream):
//!
//! - **Change messages** carry `type`, `key`, and `headers.operation` ∈
//!   {`insert`, `update`, `delete`}. Each event is parsed independently, so
//!   a single malformed event cannot poison neighbors in the same chunk.
//! - **Control messages** carry only `headers.control` ∈ {`snapshot-start`,
//!   `snapshot-end`, `reset`}. Fireline observes `snapshot-start` and
//!   `snapshot-end` passively for now; `reset` clears every projection via
//!   `StateProjection::reset` before continuing the stream.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use async_trait::async_trait;
use durable_streams::{Client, LiveMode, Offset};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

/// A change event from the state stream. Matches the state-protocol
/// change-message shape: `type`, `key`, `headers.operation` and an optional
/// `value` body.
#[derive(Debug, Deserialize)]
pub struct RawStateEnvelope {
    #[serde(rename = "type")]
    pub entity_type: String,
    pub key: String,
    pub headers: RawStateHeaders,
    #[serde(default)]
    pub value: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct RawStateHeaders {
    pub operation: String,
}

/// Control-message header. Only `control` is required; `offset` is optional.
#[derive(Debug, Deserialize)]
struct ControlHeaders {
    control: String,
}

#[async_trait]
pub trait StateProjection: Send + Sync {
    async fn apply_state_event(&self, event: &RawStateEnvelope) -> Result<()>;

    /// Discard all materialized state in response to a protocol-level
    /// `reset` control event. The default implementation is a no-op; each
    /// projection should override this if it holds mutable state that must
    /// be dropped when the upstream stream signals a reset.
    async fn reset(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct StateMaterializer {
    projections: Vec<Arc<dyn StateProjection>>,
}

pub struct StateMaterializerTask {
    up_to_date: Arc<Notify>,
    is_up_to_date: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

impl StateMaterializer {
    pub fn new(projections: Vec<Arc<dyn StateProjection>>) -> Self {
        Self { projections }
    }

    pub fn connect(&self, state_stream_url: impl Into<String>) -> StateMaterializerTask {
        let up_to_date = Arc::new(Notify::new());
        let is_up_to_date = Arc::new(AtomicBool::new(false));
        let url = state_stream_url.into();
        let materializer = self.clone();
        let notify = up_to_date.clone();
        let ready = is_up_to_date.clone();
        let handle = tokio::spawn(async move {
            consume_state_stream(url, materializer, notify, ready).await;
        });

        StateMaterializerTask {
            up_to_date,
            is_up_to_date,
            handle,
        }
    }

    async fn apply_chunk_bytes(&self, bytes: &[u8]) {
        let events: Vec<Value> = match serde_json::from_slice(bytes) {
            Ok(events) => events,
            Err(error) => {
                warn!(error = %error, "state stream chunk was not a JSON array");
                return;
            }
        };

        for event in events {
            self.apply_event(event).await;
        }
    }

    async fn apply_event(&self, event: Value) {
        match classify_event(&event) {
            EventKind::Change => match serde_json::from_value::<RawStateEnvelope>(event) {
                Ok(envelope) => {
                    if !is_supported_operation(&envelope.headers.operation) {
                        debug!(
                            operation = %envelope.headers.operation,
                            entity_type = %envelope.entity_type,
                            "skipping change event with unsupported operation"
                        );
                        return;
                    }
                    for projection in &self.projections {
                        if let Err(error) = projection.apply_state_event(&envelope).await {
                            warn!(
                                error = %error,
                                entity_type = %envelope.entity_type,
                                key = %envelope.key,
                                "projection failed to apply state event"
                            );
                        }
                    }
                }
                Err(error) => {
                    debug!(error = %error, "skip malformed change event");
                }
            },
            EventKind::Control(control) => {
                self.apply_control(&control).await;
            }
            EventKind::Unknown => {
                debug!(event = ?event, "skip unrecognized state stream event");
            }
        }
    }

    async fn apply_control(&self, control: &str) {
        match control {
            "snapshot-start" | "snapshot-end" => {
                debug!(control, "observed state stream snapshot control event");
            }
            "reset" => {
                debug!("state stream signaled reset; clearing projections");
                for projection in &self.projections {
                    if let Err(error) = projection.reset().await {
                        warn!(error = %error, "projection reset failed");
                    }
                }
            }
            other => {
                debug!(control = other, "skip unknown control event");
            }
        }
    }
}

enum EventKind {
    Change,
    Control(String),
    Unknown,
}

fn classify_event(event: &Value) -> EventKind {
    let headers = match event.get("headers") {
        Some(headers) => headers,
        None => return EventKind::Unknown,
    };
    if let Ok(control) = serde_json::from_value::<ControlHeaders>(headers.clone()) {
        return EventKind::Control(control.control);
    }
    if headers.get("operation").is_some() {
        return EventKind::Change;
    }
    EventKind::Unknown
}

fn is_supported_operation(operation: &str) -> bool {
    matches!(operation, "insert" | "update" | "delete")
}

impl StateMaterializerTask {
    pub async fn preload(&self) -> Result<()> {
        while !self.is_up_to_date.load(Ordering::SeqCst) {
            self.up_to_date.notified().await;
        }
        Ok(())
    }

    pub fn abort(self) {
        self.handle.abort();
    }
}

async fn consume_state_stream(
    url: String,
    materializer: StateMaterializer,
    up_to_date: Arc<Notify>,
    is_up_to_date: Arc<AtomicBool>,
) {
    let client = Client::new();
    let stream = client.stream(&url);

    let mut reader = match stream
        .read()
        .offset(Offset::Beginning)
        .live(LiveMode::Sse)
        .build()
    {
        Ok(reader) => reader,
        Err(error) => {
            warn!(error = %error, "build runtime materializer stream reader");
            return;
        }
    };

    loop {
        match reader.next_chunk().await {
            Ok(Some(chunk)) => {
                if !chunk.data.is_empty() {
                    materializer.apply_chunk_bytes(&chunk.data).await;
                }

                if chunk.up_to_date {
                    is_up_to_date.store(true, Ordering::SeqCst);
                    up_to_date.notify_waiters();
                }
            }
            Ok(None) => return,
            Err(error) => {
                warn!(error = %error, "runtime materializer stream read error");
                if !error.is_retryable() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct RecordingProjection {
        events: Mutex<Vec<(String, String, String)>>,
        resets: Mutex<u32>,
    }

    #[async_trait]
    impl StateProjection for RecordingProjection {
        async fn apply_state_event(&self, event: &RawStateEnvelope) -> Result<()> {
            self.events.lock().unwrap().push((
                event.entity_type.clone(),
                event.key.clone(),
                event.headers.operation.clone(),
            ));
            Ok(())
        }

        async fn reset(&self) -> Result<()> {
            *self.resets.lock().unwrap() += 1;
            self.events.lock().unwrap().clear();
            Ok(())
        }
    }

    fn chunk(events: Vec<Value>) -> Vec<u8> {
        serde_json::to_vec(&events).unwrap()
    }

    #[tokio::test]
    async fn applies_change_events_in_order() {
        let projection = Arc::new(RecordingProjection::default());
        let materializer = StateMaterializer::new(vec![projection.clone()]);
        let bytes = chunk(vec![
            serde_json::json!({
                "type": "session",
                "key": "sess-1",
                "value": {"sessionId": "sess-1"},
                "headers": {"operation": "insert"}
            }),
            serde_json::json!({
                "type": "session",
                "key": "sess-1",
                "value": {"sessionId": "sess-1"},
                "headers": {"operation": "update"}
            }),
        ]);

        materializer.apply_chunk_bytes(&bytes).await;

        let events = projection.events.lock().unwrap().clone();
        assert_eq!(
            events,
            vec![
                (
                    "session".to_string(),
                    "sess-1".to_string(),
                    "insert".to_string()
                ),
                (
                    "session".to_string(),
                    "sess-1".to_string(),
                    "update".to_string()
                ),
            ]
        );
    }

    #[tokio::test]
    async fn control_events_do_not_poison_neighboring_change_events() {
        let projection = Arc::new(RecordingProjection::default());
        let materializer = StateMaterializer::new(vec![projection.clone()]);
        let bytes = chunk(vec![
            serde_json::json!({
                "headers": {"control": "snapshot-start", "offset": "0_000"}
            }),
            serde_json::json!({
                "type": "session",
                "key": "sess-1",
                "value": {"sessionId": "sess-1"},
                "headers": {"operation": "insert"}
            }),
            serde_json::json!({
                "headers": {"control": "snapshot-end", "offset": "1_000"}
            }),
        ]);

        materializer.apply_chunk_bytes(&bytes).await;

        let events = projection.events.lock().unwrap().clone();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "session");
    }

    #[tokio::test]
    async fn malformed_change_event_does_not_drop_valid_neighbors() {
        let projection = Arc::new(RecordingProjection::default());
        let materializer = StateMaterializer::new(vec![projection.clone()]);
        let bytes = chunk(vec![
            serde_json::json!({
                "type": "session",
                "headers": {"operation": "insert"}
            }),
            serde_json::json!({
                "type": "session",
                "key": "sess-2",
                "value": {"sessionId": "sess-2"},
                "headers": {"operation": "insert"}
            }),
        ]);

        materializer.apply_chunk_bytes(&bytes).await;

        let events = projection.events.lock().unwrap().clone();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1, "sess-2");
    }

    #[tokio::test]
    async fn reset_control_triggers_projection_reset() {
        let projection = Arc::new(RecordingProjection::default());
        let materializer = StateMaterializer::new(vec![projection.clone()]);
        let bytes = chunk(vec![
            serde_json::json!({
                "type": "session",
                "key": "sess-1",
                "value": {"sessionId": "sess-1"},
                "headers": {"operation": "insert"}
            }),
            serde_json::json!({
                "headers": {"control": "reset", "offset": "42_000"}
            }),
        ]);

        materializer.apply_chunk_bytes(&bytes).await;

        assert_eq!(*projection.resets.lock().unwrap(), 1);
        assert!(projection.events.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn unsupported_operation_is_skipped() {
        let projection = Arc::new(RecordingProjection::default());
        let materializer = StateMaterializer::new(vec![projection.clone()]);
        let bytes = chunk(vec![
            serde_json::json!({
                "type": "fs_op",
                "key": "k",
                "value": {},
                "headers": {"operation": "upsert"}
            }),
            serde_json::json!({
                "type": "session",
                "key": "sess-1",
                "value": {"sessionId": "sess-1"},
                "headers": {"operation": "insert"}
            }),
        ]);

        materializer.apply_chunk_bytes(&bytes).await;

        let events = projection.events.lock().unwrap().clone();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "session");
    }
}
