use async_trait::async_trait;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTurnRecord {
    pub prompt_turn_id: String,
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildSessionEdgeInput {
    pub trace_id: Option<String>,
    pub parent_host_id: String,
    pub parent_session_id: String,
    pub parent_prompt_turn_id: String,
    pub child_host_id: String,
    pub child_session_id: String,
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

#[async_trait]
pub trait ChildSessionEdgeSink: Send + Sync {
    async fn emit_child_session_edge(&self, edge: ChildSessionEdgeInput) -> anyhow::Result<()>;
}
