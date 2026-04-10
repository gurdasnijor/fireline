use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTurnLineage {
    pub trace_id: String,
    pub prompt_turn_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct LineageTracker {
    inner: Arc<Mutex<LineageTrackerState>>,
}

#[derive(Debug, Default)]
struct LineageTrackerState {
    acp_url_to_session_id: HashMap<String, String>,
    active_turn_by_session_id: HashMap<String, ActiveTurnLineage>,
}

impl LineageTracker {
    pub fn register_session_mcp_urls(&self, session_id: &str, acp_urls: &[String]) {
        let mut state = self.inner.lock().expect("lineage tracker poisoned");
        for acp_url in acp_urls {
            state
                .acp_url_to_session_id
                .insert(acp_url.clone(), session_id.to_string());
        }
    }

    pub fn note_active_turn(&self, session_id: &str, trace_id: &str, prompt_turn_id: &str) {
        let mut state = self.inner.lock().expect("lineage tracker poisoned");
        state.active_turn_by_session_id.insert(
            session_id.to_string(),
            ActiveTurnLineage {
                trace_id: trace_id.to_string(),
                prompt_turn_id: prompt_turn_id.to_string(),
            },
        );
    }

    pub fn clear_active_turn(&self, session_id: &str, prompt_turn_id: &str) {
        let mut state = self.inner.lock().expect("lineage tracker poisoned");
        if state
            .active_turn_by_session_id
            .get(session_id)
            .is_some_and(|current| current.prompt_turn_id == prompt_turn_id)
        {
            state.active_turn_by_session_id.remove(session_id);
        }
    }

    pub fn lineage_for_acp_url(&self, acp_url: &str) -> Option<ActiveTurnLineage> {
        let state = self.inner.lock().expect("lineage tracker poisoned");
        let session_id = state.acp_url_to_session_id.get(acp_url)?;
        state.active_turn_by_session_id.get(session_id).cloned()
    }
}
