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
        self.sessions.read().unwrap().get(session_id).cloned()
    }

    pub async fn list(&self) -> Vec<SessionRecord> {
        self.sessions.read().unwrap().values().cloned().collect()
    }

    pub async fn host_spec(&self, host_key: &str) -> Option<PersistedHostSpec> {
        self.host_specs.read().unwrap().get(host_key).cloned()
    }

    pub async fn host_spec_for_session(&self, session_id: &str) -> Option<PersistedHostSpec> {
        let host_key = self
            .sessions
            .read()
            .unwrap()
            .get(session_id)
            .map(|record| record.host_key.clone())?;
        self.host_spec(&host_key).await
    }

    pub async fn host_keys(&self) -> Vec<String> {
        let mut keys = self
            .sessions
            .read()
            .unwrap()
            .values()
            .map(|record| record.host_key.clone())
            .collect::<Vec<_>>();
        keys.extend(self.host_specs.read().unwrap().keys().cloned());
        keys.sort();
        keys.dedup();
        keys
    }

    fn apply_envelope(&self, envelope: &StateEnvelope) -> Result<()> {
        let Some(operation) = envelope.change_operation() else {
            return Ok(());
        };

        match envelope.entity_type() {
            Some("session") => match operation {
                ChangeOperation::Insert | ChangeOperation::Update => {
                    let Some(value) = envelope.value.as_ref() else {
                        return Ok(());
                    };
                    let record: SessionRecord = serde_json::from_value(value.clone())?;
                    self.sessions
                        .write()
                        .unwrap()
                        .insert(record.session_id.clone(), record);
                }
                ChangeOperation::Delete => {
                    let Some(key) = envelope.key() else {
                        return Ok(());
                    };
                    self.sessions.write().unwrap().remove(key);
                }
            },
            Some("runtime_spec") => match operation {
                ChangeOperation::Insert | ChangeOperation::Update => {
                    let Some(value) = envelope.value.as_ref() else {
                        return Ok(());
                    };
                    let spec: PersistedHostSpec = serde_json::from_value(value.clone())?;
                    self.host_specs
                        .write()
                        .unwrap()
                        .insert(spec.host_key.clone(), spec);
                }
                ChangeOperation::Delete => {
                    let Some(key) = envelope.key() else {
                        return Ok(());
                    };
                    self.host_specs.write().unwrap().remove(key);
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
        self.host_specs.write().unwrap().clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SessionIndex;
    use crate::{PersistedHostSpec, projection::StateEnvelope};

    #[tokio::test]
    async fn materializes_session_rows_from_state_events() {
        let index = SessionIndex::new();
        let envelope: StateEnvelope = serde_json::from_value(serde_json::json!({
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

        index.apply_envelope(&envelope).unwrap();

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
        let host_spec_envelope: StateEnvelope = serde_json::from_value(serde_json::json!({
            "type":"runtime_spec",
            "key":"runtime:1",
            "headers":{"operation":"insert"},
            "value": host_spec,
        }))
        .unwrap();
        index.apply_envelope(&host_spec_envelope).unwrap();

        let session_envelope: StateEnvelope = serde_json::from_value(serde_json::json!({
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
        index.apply_envelope(&session_envelope).unwrap();

        let spec = index
            .host_spec_for_session("sess-1")
            .await
            .expect("host spec indexed");
        assert_eq!(spec.host_key, "runtime:1");
        assert_eq!(index.host_keys().await, vec!["runtime:1".to_string()]);
    }
}
