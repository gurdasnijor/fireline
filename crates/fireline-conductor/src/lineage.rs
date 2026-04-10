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
    active_turn_by_session_id: HashMap<String, ActiveTurnLineage>,
}

impl LineageTracker {
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

    pub fn lineage_for_session(&self, session_id: &str) -> Option<ActiveTurnLineage> {
        let state = self.inner.lock().expect("lineage tracker poisoned");
        state.active_turn_by_session_id.get(session_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::LineageTracker;

    #[test]
    fn lineage_is_scoped_to_session() {
        let tracker = LineageTracker::default();
        tracker.note_active_turn("session-a", "trace-a", "turn-a");
        tracker.note_active_turn("session-b", "trace-b", "turn-b");

        let a = tracker
            .lineage_for_session("session-a")
            .expect("session-a lineage");
        let b = tracker
            .lineage_for_session("session-b")
            .expect("session-b lineage");

        assert_eq!(a.trace_id, "trace-a");
        assert_eq!(a.prompt_turn_id, "turn-a");
        assert_eq!(b.trace_id, "trace-b");
        assert_eq!(b.prompt_turn_id, "turn-b");
    }
}
