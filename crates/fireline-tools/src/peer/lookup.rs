use async_trait::async_trait;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTurnRecord {
    pub prompt_turn_id: String,
    pub trace_id: Option<String>,
}

#[async_trait]
pub trait ActiveTurnLookup: Send + Sync {
    async fn current_turn(&self, session_id: &str) -> Option<ActiveTurnRecord>;
    async fn wait_for_current_turn(
        &self,
        session_id: &str,
        timeout: Duration,
    ) -> Option<ActiveTurnRecord>;
}
