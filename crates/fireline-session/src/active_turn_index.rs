//! Materialized in-memory lookup of the currently active prompt turn per ACP
//! session.
//!
//! This is a runtime-local projection over durable `prompt_turn` rows. It is
//! derived entirely from the state stream and is used for peer-call lineage
//! lookup without a side-channel tracker.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use fireline_tools::lookup::{ActiveTurnLookup, ActiveTurnRecord as PeerActiveTurnRecord};
use serde::Deserialize;
use tokio::sync::{Mutex, Notify, RwLock};

use crate::state_materializer::{RawStateEnvelope, StateProjection};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTurnRecord {
    pub session_id: String,
    pub prompt_turn_id: String,
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ActiveTurnIndex {
    inner: Arc<RwLock<HashMap<String, ActiveTurnRecord>>>,
    waiters: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
}

impl ActiveTurnIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get(&self, session_id: &str) -> Option<ActiveTurnRecord> {
        self.inner.read().await.get(session_id).cloned()
    }

    pub async fn wait_for(&self, session_id: &str, timeout: Duration) -> Option<ActiveTurnRecord> {
        if let Some(turn) = self.get(session_id).await {
            return Some(turn);
        }

        let notify = {
            let mut waiters = self.waiters.lock().await;
            waiters
                .entry(session_id.to_string())
                .or_insert_with(|| Arc::new(Notify::new()))
                .clone()
        };

        if let Some(turn) = self.get(session_id).await {
            return Some(turn);
        }

        tokio::time::timeout(timeout, notify.notified())
            .await
            .ok()?;
        self.get(session_id).await
    }

    async fn apply_envelope(&self, envelope: &RawStateEnvelope) -> Result<()> {
        if envelope.entity_type != "prompt_turn" {
            return Ok(());
        }

        match envelope.headers.operation.as_str() {
            "insert" | "update" => {
                let Some(value) = envelope.value.as_ref() else {
                    return Ok(());
                };
                let record: PromptTurnRecord = serde_json::from_value(value.clone())?;

                if record.state == PromptTurnState::Active {
                    let session_id = record.session_id.clone();
                    self.inner.write().await.insert(
                        session_id.clone(),
                        ActiveTurnRecord {
                            session_id: record.session_id,
                            prompt_turn_id: record.prompt_turn_id,
                            trace_id: record.trace_id,
                        },
                    );
                    if let Some(notify) = self.waiters.lock().await.get(&session_id).cloned() {
                        notify.notify_waiters();
                    }
                } else {
                    let mut inner = self.inner.write().await;
                    if inner
                        .get(&record.session_id)
                        .is_some_and(|current| current.prompt_turn_id == record.prompt_turn_id)
                    {
                        inner.remove(&record.session_id);
                    }
                }
            }
            "delete" => {
                let mut inner = self.inner.write().await;
                inner.retain(|_, current| current.prompt_turn_id != envelope.key);
            }
            _ => {}
        }

        Ok(())
    }
}

#[async_trait]
impl StateProjection for ActiveTurnIndex {
    async fn apply_state_event(&self, event: &RawStateEnvelope) -> Result<()> {
        self.apply_envelope(event).await
    }

    async fn reset(&self) -> Result<()> {
        self.inner.write().await.clear();
        self.waiters.lock().await.clear();
        Ok(())
    }
}

#[async_trait]
impl ActiveTurnLookup for ActiveTurnIndex {
    async fn current_turn(&self, session_id: &str) -> Option<PeerActiveTurnRecord> {
        self.get(session_id).await.map(|turn| PeerActiveTurnRecord {
            prompt_turn_id: turn.prompt_turn_id,
            trace_id: turn.trace_id,
        })
    }

    async fn wait_for_current_turn(
        &self,
        session_id: &str,
        timeout: Duration,
    ) -> Option<PeerActiveTurnRecord> {
        self.wait_for(session_id, timeout)
            .await
            .map(|turn| PeerActiveTurnRecord {
                prompt_turn_id: turn.prompt_turn_id,
                trace_id: turn.trace_id,
            })
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PromptTurnState {
    Queued,
    Active,
    Completed,
    CancelRequested,
    Cancelled,
    Broken,
    TimedOut,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptTurnRecord {
    prompt_turn_id: String,
    session_id: String,
    #[serde(default)]
    trace_id: Option<String>,
    state: PromptTurnState,
}

#[cfg(test)]
mod tests {
    use super::ActiveTurnIndex;
    use crate::state_materializer::RawStateEnvelope;
    use std::time::Duration;

    #[tokio::test]
    async fn tracks_active_turns_from_prompt_turn_rows() {
        let index = ActiveTurnIndex::new();
        let insert: RawStateEnvelope = serde_json::from_value(serde_json::json!({
            "type": "prompt_turn",
            "key": "turn-1",
            "headers": { "operation": "insert" },
            "value": {
                "promptTurnId": "turn-1",
                "sessionId": "sess-1",
                "traceId": "trace-1",
                "state": "active"
            }
        }))
        .unwrap();

        index.apply_envelope(&insert).await.unwrap();
        let current = index.get("sess-1").await.expect("active turn");
        assert_eq!(current.prompt_turn_id, "turn-1");
        assert_eq!(current.trace_id.as_deref(), Some("trace-1"));

        let complete: RawStateEnvelope = serde_json::from_value(serde_json::json!({
            "type": "prompt_turn",
            "key": "turn-1",
            "headers": { "operation": "update" },
            "value": {
                "promptTurnId": "turn-1",
                "sessionId": "sess-1",
                "traceId": "trace-1",
                "state": "completed"
            }
        }))
        .unwrap();

        index.apply_envelope(&complete).await.unwrap();
        assert!(index.get("sess-1").await.is_none());
    }

    #[tokio::test]
    async fn wait_for_resolves_when_active_turn_arrives() {
        let index = ActiveTurnIndex::new();
        let index_clone = index.clone();

        let waiter = tokio::spawn(async move {
            index_clone
                .wait_for("sess-2", Duration::from_millis(100))
                .await
        });

        tokio::time::sleep(Duration::from_millis(10)).await;

        let insert: RawStateEnvelope = serde_json::from_value(serde_json::json!({
            "type": "prompt_turn",
            "key": "turn-2",
            "headers": { "operation": "insert" },
            "value": {
                "promptTurnId": "turn-2",
                "sessionId": "sess-2",
                "traceId": "trace-2",
                "state": "active"
            }
        }))
        .unwrap();

        index.apply_envelope(&insert).await.unwrap();

        let turn = waiter.await.unwrap().expect("active turn should resolve");
        assert_eq!(turn.prompt_turn_id, "turn-2");
        assert_eq!(turn.trace_id.as_deref(), Some("trace-2"));
    }
}
