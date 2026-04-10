//! Materialized in-memory session index.
//!
//! Fireline persists durable `session` rows to its own state stream.
//! [`SessionIndex`] rebuilds a lookup cache by replaying that stream and then
//! following live updates. It is an in-memory materialization only; the stream
//! remains the sole durable source of truth.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use fireline_conductor::session::SessionRecord;
use tokio::sync::RwLock;

use crate::runtime_materializer::{RawStateEnvelope, StateProjection};

#[derive(Debug, Clone, Default)]
pub struct SessionIndex {
    inner: Arc<RwLock<HashMap<String, SessionRecord>>>,
}

impl SessionIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get(&self, session_id: &str) -> Option<SessionRecord> {
        self.inner.read().await.get(session_id).cloned()
    }

    pub async fn list(&self) -> Vec<SessionRecord> {
        self.inner.read().await.values().cloned().collect()
    }

    async fn apply_envelope(&self, envelope: &RawStateEnvelope) -> Result<()> {
        if envelope.entity_type != "session" {
            return Ok(());
        }

        match envelope.headers.operation.as_str() {
            "insert" | "update" => {
                let Some(value) = envelope.value.as_ref() else {
                    return Ok(());
                };
                let record: SessionRecord = serde_json::from_value(value.clone())?;
                self.inner
                    .write()
                    .await
                    .insert(record.session_id.clone(), record);
            }
            "delete" => {
                self.inner.write().await.remove(&envelope.key);
            }
            _ => {}
        }

        Ok(())
    }
}

#[async_trait]
impl StateProjection for SessionIndex {
    async fn apply_state_event(&self, event: &RawStateEnvelope) -> Result<()> {
        self.apply_envelope(event).await
    }
}

#[cfg(test)]
mod tests {
    use super::SessionIndex;
    use crate::runtime_materializer::RawStateEnvelope;

    #[tokio::test]
    async fn materializes_session_rows_from_state_events() {
        let index = SessionIndex::new();
        let envelope: RawStateEnvelope = serde_json::from_value(serde_json::json!({
            "type":"session",
            "key":"sess-1",
            "headers":{"operation":"insert"},
            "value":{
              "sessionId":"sess-1",
              "runtimeKey":"runtime:1",
              "runtimeId":"runtime-id",
              "nodeId":"node:local",
              "logicalConnectionId":"conn:1",
              "state":"active",
              "supportsLoadSession":true,
              "traceId":"trace-1",
              "parentPromptTurnId":"turn-1",
              "createdAt":1,
              "updatedAt":2,
              "lastSeenAt":3
            }
        }))
        .unwrap();

        index.apply_envelope(&envelope).await.unwrap();

        let session = index.get("sess-1").await.expect("session indexed");
        assert_eq!(session.runtime_key, "runtime:1");
        assert!(session.supports_load_session);
    }
}
