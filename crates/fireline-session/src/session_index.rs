//! Materialized in-memory session index.
//!
//! Fireline persists durable `session` rows to its own state stream.
//! [`SessionIndex`] rebuilds a lookup cache by replaying that stream and then
//! following live updates. It is an in-memory materialization only; the stream
//! remains the sole durable source of truth.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use anyhow::Result;

use crate::projection::{ChangeOperation, StateEnvelope, StreamProjection};
use crate::SessionRecord;

#[derive(Debug, Clone, Default)]
pub struct SessionIndex {
    sessions: Arc<RwLock<HashMap<String, SessionRecord>>>,
}

impl SessionIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get(&self, session_id: &str) -> Option<SessionRecord> {
        self.sessions.read().unwrap().get(session_id).cloned()
    }

    pub async fn list(&self) -> Vec<SessionRecord> {
        self.sessions.read().unwrap().values().cloned().collect()
    }

    pub async fn upsert(&self, record: SessionRecord) {
        self.sessions
            .write()
            .unwrap()
            .insert(record.session_id.to_string(), record);
    }

    fn apply_envelope(&self, envelope: &StateEnvelope) -> Result<()> {
        let Some(operation) = envelope.change_operation() else {
            return Ok(());
        };

        match envelope.entity_type() {
            Some("session_v2") => match operation {
                ChangeOperation::Insert | ChangeOperation::Update => {
                    let Some(value) = envelope.value.as_ref() else {
                        return Ok(());
                    };
                    let record: SessionRecord = serde_json::from_value(value.clone())?;
                    self.sessions
                        .write()
                        .unwrap()
                        .insert(record.session_id.to_string(), record);
                }
                ChangeOperation::Delete => {
                    let Some(key) = envelope.key() else {
                        return Ok(());
                    };
                    self.sessions.write().unwrap().remove(key);
                }
            },
            _ => {}
        }

        Ok(())
    }
}

impl StreamProjection for SessionIndex {
    fn apply(&self, event: &StateEnvelope) -> Result<()> {
        self.apply_envelope(event)
    }

    fn reset(&self) -> Result<()> {
        self.sessions.write().unwrap().clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SessionIndex;
    use crate::projection::StateEnvelope;

    #[tokio::test]
    async fn materializes_session_rows_from_state_events() {
        let index = SessionIndex::new();
        let envelope: StateEnvelope = serde_json::from_value(serde_json::json!({
            "type":"session_v2",
            "key":"sess-1",
            "headers":{"operation":"insert"},
            "value":{
              "sessionId":"sess-1",
              "state":"active",
              "supportsLoadSession":true,
              "createdAt":1,
              "updatedAt":2,
              "lastSeenAt":3
            }
        }))
        .unwrap();

        index.apply_envelope(&envelope).unwrap();

        let session = index.get("sess-1").await.expect("session indexed");
        assert_eq!(session.session_id.to_string(), "sess-1");
        assert!(session.supports_load_session);
    }

    #[tokio::test]
    async fn ignores_runtime_spec_rows() {
        let index = SessionIndex::new();
        let host_spec_envelope: StateEnvelope = serde_json::from_value(serde_json::json!({
            "type":"runtime_spec",
            "key":"runtime:1",
            "headers":{"operation":"insert"},
            "value": {},
        }))
        .unwrap();
        index.apply_envelope(&host_spec_envelope).unwrap();

        assert!(index.list().await.is_empty());
    }
}
