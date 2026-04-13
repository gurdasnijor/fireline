use std::sync::Arc;

use anyhow::Result;
use fireline_acp_ids::{RequestId, SessionId};
use serde::de::DeserializeOwned;

use crate::awakeable::{AwakeableFuture, AwakeableKey, AwakeableSubscriber};
use crate::awakeable_race::{AwakeableRaceWinner, race_awakeables};
use crate::durable_subscriber::DurableSubscriberDriver;

/// Minimal Rust-side workflow context for durable awakeable waits.
///
/// The context is intentionally small in Phase 1: it binds a state stream URL
/// and a subscriber driver, then exposes `ctx.awakeable<T>()` as imperative
/// sugar over the passive durable-subscriber substrate.
#[derive(Debug, Clone)]
pub struct WorkflowContext {
    state_stream_url: String,
    subscriber_driver: Arc<DurableSubscriberDriver>,
}

impl WorkflowContext {
    #[must_use]
    pub fn new(state_stream_url: impl Into<String>) -> Self {
        let mut subscriber_driver = DurableSubscriberDriver::new();
        subscriber_driver.register_passive(AwakeableSubscriber::new());
        Self {
            state_stream_url: state_stream_url.into(),
            subscriber_driver: Arc::new(subscriber_driver),
        }
    }

    #[must_use]
    pub fn with_subscriber_driver(
        state_stream_url: impl Into<String>,
        subscriber_driver: Arc<DurableSubscriberDriver>,
    ) -> Self {
        Self {
            state_stream_url: state_stream_url.into(),
            subscriber_driver,
        }
    }

    #[must_use]
    pub fn state_stream_url(&self) -> &str {
        &self.state_stream_url
    }

    #[must_use]
    pub fn subscriber_driver(&self) -> &Arc<DurableSubscriberDriver> {
        &self.subscriber_driver
    }

    pub fn awakeable<T>(&self, key: AwakeableKey) -> AwakeableFuture<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        AwakeableFuture::new(
            self.state_stream_url.clone(),
            Arc::clone(&self.subscriber_driver),
            key,
        )
    }

    pub fn session_awakeable<T>(&self, session_id: SessionId) -> AwakeableFuture<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.awakeable(AwakeableKey::session(session_id))
    }

    pub fn prompt_awakeable<T>(
        &self,
        session_id: SessionId,
        request_id: RequestId,
    ) -> AwakeableFuture<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        self.awakeable(AwakeableKey::prompt(session_id, request_id))
    }

    pub async fn race<T, I>(&self, awakeables: I) -> Result<AwakeableRaceWinner<T>>
    where
        I: IntoIterator<Item = AwakeableFuture<T>>,
        T: DeserializeOwned + Send + 'static,
    {
        race_awakeables(awakeables).await
    }
}
