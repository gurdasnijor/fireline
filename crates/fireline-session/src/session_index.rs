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
use tokio::sync::RwLock;

use crate::state_materializer::{RawStateEnvelope, StateProjection};
use crate::{PersistedHostSpec, SessionRecord};

#[derive(Debug, Clone, Default)]
pub struct SessionIndex {
    sessions: Arc<RwLock<HashMap<String, SessionRecord>>>,
    host_specs: Arc<RwLock<HashMap<String, PersistedHostSpec>>>,
}

impl SessionIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get(&self, session_id: &str) -> Option<SessionRecord> {
        self.sessions.read().await.get(session_id).cloned()
    }

    pub async fn list(&self) -> Vec<SessionRecord> {
        self.sessions.read().await.values().cloned().collect()
    }

    pub async fn host_spec(&self, host_key: &str) -> Option<PersistedHostSpec> {
        self.host_specs.read().await.get(host_key).cloned()
    }

    pub async fn host_spec_for_session(&self, session_id: &str) -> Option<PersistedHostSpec> {
        let host_key = self
            .sessions
            .read()
            .await
            .get(session_id)
            .map(|record| record.host_key.clone())?;
        self.host_spec(&host_key).await
    }

    pub async fn host_keys(&self) -> Vec<String> {
        let mut keys = self
            .sessions
            .read()
            .await
            .values()
            .map(|record| record.host_key.clone())
            .collect::<Vec<_>>();
        keys.extend(self.host_specs.read().await.keys().cloned());
        keys.sort();
        keys.dedup();
        keys
    }

    async fn apply_envelope(&self, envelope: &RawStateEnvelope) -> Result<()> {
        match envelope.entity_type.as_str() {
            "session" => match envelope.headers.operation.as_str() {
                "insert" | "update" => {
                    let Some(value) = envelope.value.as_ref() else {
                        return Ok(());
                    };
                    let record: SessionRecord = serde_json::from_value(value.clone())?;
                    self.sessions
                        .write()
                        .await
                        .insert(record.session_id.clone(), record);
                }
                "delete" => {
                    self.sessions.write().await.remove(&envelope.key);
                }
                _ => {}
            },
            "runtime_spec" => match envelope.headers.operation.as_str() {
                "insert" | "update" => {
                    let Some(value) = envelope.value.as_ref() else {
                        return Ok(());
                    };
                    let spec: PersistedHostSpec = serde_json::from_value(value.clone())?;
                    self.host_specs
                        .write()
                        .await
                        .insert(spec.host_key.clone(), spec);
                }
                "delete" => {
                    self.host_specs.write().await.remove(&envelope.key);
                }
                _ => {}
            },
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

    async fn reset(&self) -> Result<()> {
        self.sessions.write().await.clear();
        self.host_specs.write().await.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SessionIndex;
    use crate::{PersistedHostSpec, state_materializer::RawStateEnvelope};

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
        assert_eq!(session.host_key, "runtime:1");
        assert!(session.supports_load_session);
    }

    #[tokio::test]
    async fn materializes_host_spec_rows_and_joins_through_session_records() {
        let index = SessionIndex::new();
        let host_spec = PersistedHostSpec::new(
            "runtime:1",
            "node:test",
            serde_json::json!({
                "provider": "local",
                "host": "127.0.0.1",
                "port": 0,
                "name": "resume-test",
                "agentCommand": ["/bin/echo"],
                "resources": [],
                "stateStream": "state-test",
                "streamStorage": null,
                "peerDirectoryPath": "/tmp/peers.toml",
                "topology": { "components": [] }
            }),
        );
        let host_spec_envelope: RawStateEnvelope = serde_json::from_value(serde_json::json!({
            "type":"runtime_spec",
            "key":"runtime:1",
            "headers":{"operation":"insert"},
            "value": host_spec,
        }))
        .unwrap();
        index.apply_envelope(&host_spec_envelope).await.unwrap();

        let session_envelope: RawStateEnvelope = serde_json::from_value(serde_json::json!({
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
        index.apply_envelope(&session_envelope).await.unwrap();

        let spec = index
            .host_spec_for_session("sess-1")
            .await
            .expect("host spec indexed");
        assert_eq!(spec.host_key, "runtime:1");
        assert_eq!(index.host_keys().await, vec!["runtime:1".to_string()]);
    }
}
