//! Per-session budget gate.
//!
//! Maintains per-session counters (tokens, tool calls, wall clock)
//! and terminates the current `session/prompt` — or lets it through
//! with a warning — when a configured limit is crossed. The
//! prompt-token counting path is fully wired: the component
//! intercepts `PromptRequest` on the client-facing side of the
//! proxy, extracts the text blocks, increments the session's
//! counter, and refuses the request when the configured ceiling
//! is exceeded.
//!
//! # What's wired vs TODO
//!
//! - **`record_prompt_tokens`** is called on every `session/prompt`
//!   by the `ConnectTo<Conductor>` impl below.
//! - **`record_tool_call`** is *not* yet called on agent→MCP tool
//!   dispatches. Tool calls travel as MCP-over-ACP and don't
//!   present a clean proxy-level hook today. The counter method
//!   is fully implemented and unit-tested so it can be wired as
//!   soon as the SDK exposes the interception point.
//! - **Token counting** is `ceil(chars / 4)` — approximate enough
//!   for coarse budgets, not a substitute for a real tokenizer.
//!   TODO: swap in `tiktoken-rs` or an adapter over the model
//!   provider's tokenizer once there's a consumer that needs the
//!   precision.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sacp::schema::{ContentBlock, PromptRequest};
use sacp::{Agent, Client, ConnectTo, Proxy};

#[derive(Clone, Debug, Default)]
pub struct BudgetConfig {
    pub max_tokens: Option<u64>,
    pub max_tool_calls: Option<u64>,
    pub max_duration: Option<Duration>,
    pub on_exceeded: BudgetAction,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BudgetAction {
    /// Refuse the current prompt turn with a budget-exceeded error.
    /// The agent sees a failure response for the offending request.
    #[default]
    TerminateTurn,
    /// Forward the current request but record the exceeded state
    /// so later reads of `is_exceeded` return `true`.
    DenyFurtherCalls,
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
    /// session has now exceeded any configured limit.
    pub fn record_prompt_tokens(&self, session_id: &str, text: &str) -> bool {
        let approx = approximate_tokens(text);
        let mut state = self.state.lock().expect("budget state poisoned");
        let entry = state
            .entry(session_id.to_string())
            .or_insert_with(SessionBudgetState::new);
        entry.tokens_used = entry.tokens_used.saturating_add(approx);
        self.is_exceeded_entry(entry)
    }

    /// Record that a tool call happened in the given session.
    /// Returns `true` if the session has now exceeded any
    /// configured limit.
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

    /// Extract the joined text from all `ContentBlock::Text`
    /// entries of a `PromptRequest`. Used by the interceptor to
    /// derive a token count.
    pub fn join_prompt_text(request: &PromptRequest) -> String {
        request
            .prompt
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ")
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
        let this = self.clone();
        sacp::Proxy
            .builder()
            .name("fireline-budget")
            .on_receive_request_from(
                Client,
                {
                    let this = this.clone();
                    async move |request: PromptRequest, responder, cx| {
                        let session_id = request.session_id.to_string();
                        let prompt_text = BudgetComponent::join_prompt_text(&request);
                        let exceeded = this.record_prompt_tokens(&session_id, &prompt_text);
                        if exceeded {
                            match this.config.on_exceeded {
                                BudgetAction::TerminateTurn => {
                                    return Err(sacp::util::internal_error(format!(
                                        "budget_gate: session {session_id} exceeded its budget"
                                    )));
                                }
                                BudgetAction::DenyFurtherCalls => {
                                    // Fall through — this request still runs,
                                    // but subsequent `is_exceeded` reads will
                                    // return `true`, and a future tool-call
                                    // interceptor can refuse based on that.
                                }
                            }
                        }
                        cx.send_request_to(Agent, request)
                            .forward_response_to(responder)
                    }
                },
                sacp::on_receive_request!(),
            )
            .connect_to(client)
            .await
    }
}

/// Approximate token count as `ceil(chars / 4)`. This is a
/// deliberately crude placeholder — good enough for order-of-
/// magnitude budget decisions, wrong enough that nobody should
/// use it for billing. Swap for a real tokenizer once there's a
/// concrete consumer that needs the precision.
fn approximate_tokens(text: &str) -> u64 {
    (text.chars().count() as u64).div_ceil(4)
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
        assert!(!component.record_prompt_tokens("s1", "hello"));
        assert!(!component.record_prompt_tokens("s1", "world"));
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

    #[test]
    fn join_prompt_text_concats_text_blocks_only() {
        let request = PromptRequest::new(
            sacp::schema::SessionId::from("sess-1"),
            vec![
                ContentBlock::from("hello".to_string()),
                ContentBlock::from("world".to_string()),
            ],
        );
        let joined = BudgetComponent::join_prompt_text(&request);
        assert_eq!(joined, "hello world");
    }

    #[test]
    fn approximate_tokens_rounds_up() {
        assert_eq!(approximate_tokens(""), 0);
        assert_eq!(approximate_tokens("a"), 1);
        assert_eq!(approximate_tokens("aaaa"), 1);
        assert_eq!(approximate_tokens("aaaaa"), 2);
    }
}
