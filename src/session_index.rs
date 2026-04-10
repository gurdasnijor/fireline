//! Materialized in-memory session index.
//!
//! Fireline persists durable `session` rows to its own state stream.
//! [`SessionIndex`] rebuilds a lookup cache by replaying that stream and then
//! following live updates. It is an in-memory materialization only; the stream
//! remains the sole durable source of truth.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use durable_streams::{Client, LiveMode, Offset};
use fireline_conductor::session::SessionRecord;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{Notify, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

#[derive(Debug, Clone, Default)]
pub struct SessionIndex {
    inner: Arc<RwLock<HashMap<String, SessionRecord>>>,
}

pub struct SessionIndexTask {
    up_to_date: Arc<Notify>,
    handle: JoinHandle<()>,
}

impl SessionIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn connect(&self, state_stream_url: impl Into<String>) -> SessionIndexTask {
        let up_to_date = Arc::new(Notify::new());
        let url = state_stream_url.into();
        let index = self.clone();
        let notify = up_to_date.clone();
        let handle = tokio::spawn(async move {
            consume_session_rows(url, index, notify).await;
        });

        SessionIndexTask { up_to_date, handle }
    }

    pub async fn get(&self, session_id: &str) -> Option<SessionRecord> {
        self.inner.read().await.get(session_id).cloned()
    }

    pub async fn list(&self) -> Vec<SessionRecord> {
        self.inner.read().await.values().cloned().collect()
    }

    async fn apply_chunk_bytes(&self, bytes: &[u8]) -> Result<()> {
        let events = serde_json::from_slice::<Vec<Value>>(bytes)?;
        for event in events {
            self.apply_state_event(event).await?;
        }
        Ok(())
    }

    async fn apply_state_event(&self, event: Value) -> Result<()> {
        let envelope: RawStateEnvelope =
            serde_json::from_value(event).map_err(anyhow::Error::from)?;

        if envelope.entity_type != "session" {
            return Ok(());
        }

        match envelope.headers.operation.as_str() {
            "insert" | "update" => {
                let Some(value) = envelope.value else {
                    return Ok(());
                };
                let record: SessionRecord = serde_json::from_value(value)?;
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

impl SessionIndexTask {
    pub async fn preload(&self) -> Result<()> {
        self.up_to_date.notified().await;
        Ok(())
    }

    pub fn abort(self) {
        self.handle.abort();
    }
}

async fn consume_session_rows(url: String, index: SessionIndex, up_to_date: Arc<Notify>) {
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
            warn!(error = %error, "build session index stream reader");
            return;
        }
    };

    loop {
        match reader.next_chunk().await {
            Ok(Some(chunk)) => {
                if !chunk.data.is_empty()
                    && let Err(error) = index.apply_chunk_bytes(&chunk.data).await
                {
                    debug!(error = %error, "skip unparseable session index chunk");
                }

                if chunk.up_to_date {
                    up_to_date.notify_waiters();
                }
            }
            Ok(None) => return,
            Err(error) => {
                warn!(error = %error, "session index stream read error");
                if !error.is_retryable() {
                    return;
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawStateEnvelope {
    #[serde(rename = "type")]
    entity_type: String,
    key: String,
    headers: RawStateHeaders,
    #[serde(default)]
    value: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct RawStateHeaders {
    operation: String,
}

#[cfg(test)]
mod tests {
    use super::SessionIndex;

    #[tokio::test]
    async fn materializes_session_rows_from_state_events() {
        let index = SessionIndex::new();
        let payload = br#"[
          {
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
          }
        ]"#;

        index.apply_chunk_bytes(payload).await.unwrap();

        let session = index.get("sess-1").await.expect("session indexed");
        assert_eq!(session.runtime_key, "runtime:1");
        assert!(session.supports_load_session);
    }
}
