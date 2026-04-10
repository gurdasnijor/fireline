//! Shared runtime-side durable state materializer.
//!
//! One runtime should keep one durable-streams subscriber / replay loop and
//! fan out decoded `STATE-PROTOCOL` events to narrow in-memory projections.
//! The projections remain replaceable and in-memory only; the durable state
//! stream remains the sole durable source of truth.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use durable_streams::{Client, LiveMode, Offset};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

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

#[async_trait]
pub trait StateProjection: Send + Sync {
    async fn apply_state_event(&self, event: &RawStateEnvelope) -> Result<()>;
}

#[derive(Clone, Default)]
pub struct RuntimeMaterializer {
    projections: Vec<Arc<dyn StateProjection>>,
}

pub struct RuntimeMaterializerTask {
    up_to_date: Arc<Notify>,
    handle: JoinHandle<()>,
}

impl RuntimeMaterializer {
    pub fn new(projections: Vec<Arc<dyn StateProjection>>) -> Self {
        Self { projections }
    }

    pub fn connect(&self, state_stream_url: impl Into<String>) -> RuntimeMaterializerTask {
        let up_to_date = Arc::new(Notify::new());
        let url = state_stream_url.into();
        let materializer = self.clone();
        let notify = up_to_date.clone();
        let handle = tokio::spawn(async move {
            consume_state_stream(url, materializer, notify).await;
        });

        RuntimeMaterializerTask { up_to_date, handle }
    }

    async fn apply_chunk_bytes(&self, bytes: &[u8]) -> Result<()> {
        let events = serde_json::from_slice::<Vec<Value>>(bytes)?;
        for event in events {
            let envelope: RawStateEnvelope =
                serde_json::from_value(event).map_err(anyhow::Error::from)?;
            for projection in &self.projections {
                projection.apply_state_event(&envelope).await?;
            }
        }
        Ok(())
    }
}

impl RuntimeMaterializerTask {
    pub async fn preload(&self) -> Result<()> {
        self.up_to_date.notified().await;
        Ok(())
    }

    pub fn abort(self) {
        self.handle.abort();
    }
}

async fn consume_state_stream(
    url: String,
    materializer: RuntimeMaterializer,
    up_to_date: Arc<Notify>,
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
                if !chunk.data.is_empty()
                    && let Err(error) = materializer.apply_chunk_bytes(&chunk.data).await
                {
                    debug!(error = %error, "skip unparseable runtime materializer chunk");
                }

                if chunk.up_to_date {
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
