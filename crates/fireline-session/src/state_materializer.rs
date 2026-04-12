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
//!   {`insert`, `update`, `delete`} with optional `old_value`, `txid`, and
//!   `timestamp`. Each event is parsed independently, so a single malformed
//!   event cannot poison neighbors in the same chunk.
//! - **Control messages** carry only `headers.control` ∈ {`snapshot-start`,
//!   `snapshot-end`, `reset`} plus optional `headers.offset`. Fireline
//!   observes `snapshot-start` and `snapshot-end` passively for now; `reset`
//!   clears every projection via `StreamProjection::reset` before continuing
//!   the stream.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, anyhow};
use durable_streams::{Client, LiveMode, Offset};
use serde_json::Value;
use tokio::sync::{Mutex, Notify};
use tokio::task::{AbortHandle, JoinHandle};
use tracing::{debug, warn};

use crate::projection::{ControlKind, StateEnvelope, StreamProjection};

#[derive(Clone, Default)]
pub struct StateMaterializer {
    projections: Vec<Arc<dyn StreamProjection>>,
}

pub struct StateMaterializerTask {
    up_to_date: Arc<Notify>,
    is_up_to_date: Arc<AtomicBool>,
    abort_handle: AbortHandle,
    handle: Arc<Mutex<JoinHandle<Result<()>>>>,
}

impl StateMaterializer {
    pub fn new(projections: Vec<Arc<dyn StreamProjection>>) -> Self {
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
            consume_state_stream(url, materializer, notify, ready).await
        });
        let abort_handle = handle.abort_handle();

        StateMaterializerTask {
            up_to_date,
            is_up_to_date,
            abort_handle,
            handle: Arc::new(Mutex::new(handle)),
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
        let envelope = match serde_json::from_value::<StateEnvelope>(event.clone()) {
            Ok(envelope) => envelope,
            Err(error) => {
                debug!(error = %error, "skip malformed state event");
                return;
            }
        };

        match classify_event(&envelope) {
            EventKind::Change => {
                let entity_type = envelope.entity_type().unwrap_or("<missing>");
                let key = envelope.key().unwrap_or("<missing>");
                let operation = envelope.change_operation();

                for projection in &self.projections {
                    if let Err(error) = projection.apply(&envelope) {
                        warn!(
                            error = %error,
                            entity_type,
                            key,
                            operation = ?operation,
                            "projection failed to apply state event"
                        );
                    }
                }
            }
            EventKind::Control(control) => {
                self.apply_control(control);
            }
            EventKind::Unknown => {
                debug!(event = ?event, "skip unrecognized state stream event");
            }
        }
    }

    fn apply_control(&self, control: ControlKind) {
        match control {
            ControlKind::SnapshotStart | ControlKind::SnapshotEnd => {
                debug!(control = ?control, "observed state stream snapshot control event");
            }
            ControlKind::Reset => {
                debug!("state stream signaled reset; clearing projections");
                for projection in &self.projections {
                    if let Err(error) = projection.reset() {
                        warn!(error = %error, "projection reset failed");
                    }
                }
            }
        }
    }
}

enum EventKind {
    Change,
    Control(ControlKind),
    Unknown,
}

fn classify_event(event: &StateEnvelope) -> EventKind {
    if let Some(control) = event.control_kind() {
        return EventKind::Control(control);
    }
    if event.is_change() {
        return EventKind::Change;
    }
    EventKind::Unknown
}

impl StateMaterializerTask {
    pub async fn preload(&self) -> Result<()> {
        while !self.is_up_to_date.load(Ordering::SeqCst) {
            tokio::select! {
                _ = self.up_to_date.notified() => {}
                join_result = async {
                    let mut handle = self.handle.lock().await;
                    (&mut *handle).await
                } => {
                    match join_result {
                        Ok(Ok(())) => {
                            return Err(anyhow!(
                                "state materializer worker exited before reaching the live edge"
                            ));
                        }
                        Ok(Err(error)) => {
                            return Err(error)
                                .context("state materializer worker exited before preload completed");
                        }
                        Err(error) => {
                            return Err(anyhow::Error::from(error))
                                .context("join state materializer worker before preload completed");
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn abort(self) {
        self.abort_handle.abort();
    }
}

async fn consume_state_stream(
    url: String,
    materializer: StateMaterializer,
    up_to_date: Arc<Notify>,
    is_up_to_date: Arc<AtomicBool>,
) -> Result<()> {
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
            return Err(anyhow::Error::from(error))
                .context("build runtime materializer stream reader");
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
            Ok(None) => return Ok(()),
            Err(error) => {
                warn!(error = %error, "runtime materializer stream read error");
                if !error.is_retryable() {
                    return Err(anyhow::Error::from(error))
                        .context("runtime materializer stream read error");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::projection::{ChangeOperation, StreamProjection};

    #[derive(Default)]
    struct RecordingProjection {
        events: Mutex<Vec<(String, String, String)>>,
        resets: Mutex<u32>,
    }

    impl StreamProjection for RecordingProjection {
        fn apply(&self, event: &StateEnvelope) -> Result<()> {
            self.events.lock().unwrap().push((
                event.entity_type.clone().unwrap(),
                event.key.clone().unwrap(),
                format!("{:?}", event.headers.operation.clone().unwrap()).to_lowercase(),
            ));
            Ok(())
        }

        fn reset(&self) -> Result<()> {
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

    #[test]
    fn state_envelope_deserializes_protocol_optional_fields() {
        let envelope: StateEnvelope = serde_json::from_value(serde_json::json!({
            "type": "session",
            "key": "sess-1",
            "value": {"sessionId": "sess-1"},
            "old_value": {"sessionId": "sess-0"},
            "headers": {
                "operation": "update",
                "txid": "tx-1",
                "timestamp": "2025-01-15T10:35:00Z"
            }
        }))
        .unwrap();

        assert_eq!(envelope.entity_type.as_deref(), Some("session"));
        assert_eq!(envelope.key.as_deref(), Some("sess-1"));
        assert_eq!(envelope.headers.operation, Some(ChangeOperation::Update));
        assert_eq!(envelope.headers.txid.as_deref(), Some("tx-1"));
        assert_eq!(
            envelope.headers.timestamp.as_deref(),
            Some("2025-01-15T10:35:00Z")
        );
        assert!(envelope.old_value.is_some());
    }

    #[tokio::test]
    async fn preload_errors_if_worker_exits_before_live_edge() {
        let handle = tokio::spawn(async { Ok::<(), anyhow::Error>(()) });
        let task = StateMaterializerTask {
            up_to_date: Arc::new(Notify::new()),
            is_up_to_date: Arc::new(AtomicBool::new(false)),
            abort_handle: handle.abort_handle(),
            handle: Arc::new(tokio::sync::Mutex::new(handle)),
        };

        let error = task
            .preload()
            .await
            .expect_err("preload should fail when the worker exits before the live edge");
        assert!(
            error
                .to_string()
                .contains("exited before reaching the live edge"),
            "expected live-edge exit error, got: {error:#}"
        );
    }
}
