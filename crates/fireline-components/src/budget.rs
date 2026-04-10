//! Per-session budget gate — SKETCH.
//!
//! Intended shape: maintain per-session counters (tokens, tool calls,
//! wall clock) and terminate the turn (or deny further calls) when a
//! configured limit is crossed.
//!
//! # SKETCH STATUS
//!
//! - Counter state, `record_prompt_tokens`, `record_tool_call`, and
//!   `is_exceeded` are fully implemented and unit-tested.
//! - Token counting is a crude `ceil(chars/4)` placeholder — TODO:
//!   switch to a real tokenizer.
//! - The `ConnectTo<Conductor>` impl is a pass-through proxy; the
//!   interception hook that actually calls the counter methods on
//!   each `session/prompt` request and tool call is TODO. Same SDK
//!   gap as `approval.rs`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sacp::{ConnectTo, Proxy};

#[derive(Clone, Debug, Default)]
pub struct BudgetConfig {
    pub max_tokens: Option<u64>,
    pub max_tool_calls: Option<u64>,
    pub max_duration: Option<Duration>,
    pub on_exceeded: BudgetAction,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BudgetAction {
    /// End the current turn with a budget-exceeded error.
    #[default]
    TerminateTurn,
    /// Deny further tool calls but let the current turn finish.
    DenyFurtherCalls,
    // TODO: RequireApproval — compose with ApprovalGateComponent.
}

#[derive(Debug)]
struct SessionBudgetState {
    tokens_used: u64,
    tool_calls_made: u64,
    started_at: Instant,
}

impl SessionBudgetState {
    fn new() -> Self {
        Self {
            tokens_used: 0,
            tool_calls_made: 0,
            started_at: Instant::now(),
        }
    }
}

#[derive(Clone)]
pub struct BudgetComponent {
    config: BudgetConfig,
    state: Arc<Mutex<HashMap<String, SessionBudgetState>>>,
}

impl BudgetComponent {
    pub fn new(config: BudgetConfig) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Record token usage against a session. Returns `true` if the
    /// session has now exceeded any configured limit. Token estimate
    /// is `ceil(chars / 4)` — TODO: swap for a real tokenizer.
    pub fn record_prompt_tokens(&self, session_id: &str, text: &str) -> bool {
        let approx = (text.chars().count() as u64).div_ceil(4);
        let mut state = self.state.lock().expect("budget state poisoned");
        let entry = state
            .entry(session_id.to_string())
            .or_insert_with(SessionBudgetState::new);
        entry.tokens_used = entry.tokens_used.saturating_add(approx);
        self.is_exceeded_entry(entry)
    }

    /// Record that a tool call happened in the given session. Returns
    /// `true` if the session has now exceeded any configured limit.
    pub fn record_tool_call(&self, session_id: &str) -> bool {
        let mut state = self.state.lock().expect("budget state poisoned");
        let entry = state
            .entry(session_id.to_string())
            .or_insert_with(SessionBudgetState::new);
        entry.tool_calls_made = entry.tool_calls_made.saturating_add(1);
        self.is_exceeded_entry(entry)
    }

    /// Non-mutating check: has this session exceeded its budget?
    pub fn is_exceeded(&self, session_id: &str) -> bool {
        let state = self.state.lock().expect("budget state poisoned");
        state
            .get(session_id)
            .map(|entry| self.is_exceeded_entry(entry))
            .unwrap_or(false)
    }

    fn is_exceeded_entry(&self, entry: &SessionBudgetState) -> bool {
        if let Some(max) = self.config.max_tokens {
            if entry.tokens_used > max {
                return true;
            }
        }
        if let Some(max) = self.config.max_tool_calls {
            if entry.tool_calls_made > max {
                return true;
            }
        }
        if let Some(max) = self.config.max_duration {
            if entry.started_at.elapsed() > max {
                return true;
            }
        }
        false
    }
}

impl ConnectTo<sacp::Conductor> for BudgetComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        let _this = self;
        // TODO: intercept `session/prompt` to call
        // `_this.record_prompt_tokens(session_id, text)`, intercept
        // tool-call traffic to call `_this.record_tool_call(session_id)`,
        // and on `is_exceeded` invoke the `on_exceeded` action.
        sacp::Proxy
            .builder()
            .name("fireline-budget")
            .connect_to(client)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_limit_exceeds() {
        let component = BudgetComponent::new(BudgetConfig {
            max_tokens: Some(10),
            ..Default::default()
        });
        // "hello" is 5 chars → ceil(5/4) = 2 tokens
        assert!(!component.record_prompt_tokens("s1", "hello"));
        // another 5 chars → 2 tokens, total 4
        assert!(!component.record_prompt_tokens("s1", "world"));
        // a longer message → cumulative > 10
        assert!(component.record_prompt_tokens(
            "s1",
            "this is a substantially longer prompt that should blow the budget",
        ));
    }

    #[test]
    fn tool_call_limit_exceeds() {
        let component = BudgetComponent::new(BudgetConfig {
            max_tool_calls: Some(2),
            ..Default::default()
        });
        assert!(!component.record_tool_call("s1"));
        assert!(!component.record_tool_call("s1"));
        assert!(component.record_tool_call("s1"));
    }

    #[test]
    fn sessions_are_isolated() {
        let component = BudgetComponent::new(BudgetConfig {
            max_tool_calls: Some(1),
            ..Default::default()
        });
        assert!(!component.record_tool_call("s1"));
        assert!(!component.record_tool_call("s2"));
        assert!(component.record_tool_call("s1"));
        assert!(component.record_tool_call("s2"));
    }

    #[test]
    fn is_exceeded_false_for_unknown_session() {
        let component = BudgetComponent::new(BudgetConfig {
            max_tokens: Some(1),
            ..Default::default()
        });
        assert!(!component.is_exceeded("never-seen"));
    }
}
