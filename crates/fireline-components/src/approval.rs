//! Approval gate proxy.
//!
//! Intercepts `session/prompt` requests flowing from the client
//! and runs each one through a configured policy. Matching
//! policies can either:
//!
//! - **Deny** the request outright, returning a tool-level error
//!   so the agent sees a failure instead of the requested action
//! - **RequireApproval** — forward the request only after a
//!   human approver signals consent (scaffolded below as a TODO,
//!   because the SDK doesn't yet expose a clean proxy-level hook
//!   for `session/request_permission` round-trips initiated from
//!   inside an `on_receive_request_from` handler)
//!
//! # Why prompt-level, not tool-call-level
//!
//! The obvious gate point is an individual tool call — "ask
//! before the agent runs `shell`." That would require
//! intercepting agent→MCP tool dispatches, which today don't
//! present a cleanly typed ACP proxy hook (tool calls travel as
//! MCP-over-ACP). Until that hook lands upstream in
//! `agent-client-protocol-core`, this component gates at the
//! *prompt* level: scan the user's prompt for dangerous
//! keywords, risky file paths, or other policy-relevant
//! substrings, and refuse or escalate before the agent ever sees
//! the request. That's a strictly weaker guarantee than tool-
//! call gating, but it's real, it composes with the existing
//! proxy chain, and it maps cleanly to concrete user policies.
//!
//! Tool-name policies are retained on `ApprovalMatch` for
//! eventual use once the SDK supports tool-call interception;
//! the pattern matcher already handles them in tests.

use std::sync::Arc;

use sacp::schema::{ContentBlock, PromptRequest};
use sacp::{Agent, Client, ConnectTo, Proxy};

#[derive(Clone, Default)]
pub struct ApprovalConfig {
    pub policies: Vec<ApprovalPolicy>,
}

#[derive(Clone)]
pub struct ApprovalPolicy {
    pub match_rule: ApprovalMatch,
    pub action: ApprovalAction,
    /// Human-readable reason included in denial messages.
    pub reason: String,
}

#[derive(Clone)]
pub enum ApprovalMatch {
    /// The prompt text contains this case-insensitive substring.
    PromptContains { needle: String },
    /// Exact match by tool name — retained for future use once
    /// tool-call interception is possible. Not yet evaluated
    /// anywhere in the prompt-level gate.
    Tool { name: String },
    /// Prefix match on the tool name.
    ToolPrefix { prefix: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalAction {
    /// Pause the request and ask the user via
    /// `session/request_permission`. **TODO**: currently falls
    /// through to forwarding; the round-trip is scaffolded but
    /// not wired.
    RequireApproval,
    /// Refuse the request with a gate error the agent will see.
    Deny,
}

impl ApprovalMatch {
    pub fn matches_tool(&self, tool_name: &str) -> bool {
        match self {
            ApprovalMatch::Tool { name } => name == tool_name,
            ApprovalMatch::ToolPrefix { prefix } => tool_name.starts_with(prefix.as_str()),
            ApprovalMatch::PromptContains { .. } => false,
        }
    }

    pub fn matches_prompt(&self, prompt_text: &str) -> bool {
        match self {
            ApprovalMatch::PromptContains { needle } => {
                prompt_text.to_ascii_lowercase().contains(&needle.to_ascii_lowercase())
            }
            ApprovalMatch::Tool { .. } | ApprovalMatch::ToolPrefix { .. } => false,
        }
    }
}

impl ApprovalConfig {
    /// Return the first matching policy for a given tool name, or
    /// `None` if no policy applies (implicit allow).
    pub fn policy_for_tool(&self, tool_name: &str) -> Option<&ApprovalPolicy> {
        self.policies
            .iter()
            .find(|p| p.match_rule.matches_tool(tool_name))
    }

    /// Return the first matching policy against the full joined
    /// prompt text.
    pub fn policy_for_prompt(&self, prompt_text: &str) -> Option<&ApprovalPolicy> {
        self.policies
            .iter()
            .find(|p| p.match_rule.matches_prompt(prompt_text))
    }
}

#[derive(Clone)]
pub struct ApprovalGateComponent {
    config: Arc<ApprovalConfig>,
}

impl ApprovalGateComponent {
    pub fn new(config: ApprovalConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }

    pub fn config(&self) -> &ApprovalConfig {
        &self.config
    }

    /// Extract the joined prompt text from all `ContentBlock::Text`
    /// entries on a `PromptRequest`. Non-text blocks are ignored.
    /// This is the input to prompt-level policy matching.
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
}

impl ConnectTo<sacp::Conductor> for ApprovalGateComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        let config = self.config.clone();
        sacp::Proxy
            .builder()
            .name("fireline-approval")
            .on_receive_request_from(
                Client,
                {
                    let config = config.clone();
                    async move |request: PromptRequest, responder, cx| {
                        let prompt_text = ApprovalGateComponent::join_prompt_text(&request);
                        if let Some(policy) = config.policy_for_prompt(&prompt_text) {
                            match policy.action {
                                ApprovalAction::Deny => {
                                    return Err(sacp::util::internal_error(format!(
                                        "approval_gate denied prompt: {}",
                                        policy.reason
                                    )));
                                }
                                ApprovalAction::RequireApproval => {
                                    // TODO: issue `session/request_permission`
                                    // back toward the client via
                                    // `cx.send_request_to(Client, RequestPermissionRequest::new(...))`,
                                    // await the human's choice, then either forward or
                                    // reject based on the outcome. Until that path is
                                    // pinned, fall through to forwarding so the gate
                                    // behaves as "log-and-allow."
                                    let _ = policy;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn deny_policy(needle: &str, reason: &str) -> ApprovalPolicy {
        ApprovalPolicy {
            match_rule: ApprovalMatch::PromptContains {
                needle: needle.to_string(),
            },
            action: ApprovalAction::Deny,
            reason: reason.to_string(),
        }
    }

    #[test]
    fn prompt_contains_is_case_insensitive() {
        let rule = ApprovalMatch::PromptContains {
            needle: "rm -rf".to_string(),
        };
        assert!(rule.matches_prompt("please run RM -RF /tmp"));
        assert!(rule.matches_prompt("rm -rf something"));
        assert!(!rule.matches_prompt("remove the temp directory carefully"));
    }

    #[test]
    fn tool_rules_dont_match_prompt_text() {
        let rule = ApprovalMatch::Tool {
            name: "shell".to_string(),
        };
        assert!(!rule.matches_prompt("please run shell commands"));
    }

    #[test]
    fn policy_lookup_uses_first_match() {
        let config = ApprovalConfig {
            policies: vec![
                deny_policy("rm -rf", "destructive recursive delete"),
                deny_policy("drop table", "destructive DB operation"),
            ],
        };
        let policy = config.policy_for_prompt("help me write a DROP TABLE query").unwrap();
        assert_eq!(policy.reason, "destructive DB operation");
        assert!(config.policy_for_prompt("help me refactor").is_none());
    }

    #[test]
    fn exact_tool_match_still_works() {
        let rule = ApprovalMatch::Tool {
            name: "shell".to_string(),
        };
        assert!(rule.matches_tool("shell"));
        assert!(!rule.matches_tool("shell_run"));
    }

    #[test]
    fn prefix_tool_match_still_works() {
        let rule = ApprovalMatch::ToolPrefix {
            prefix: "fs/".to_string(),
        };
        assert!(rule.matches_tool("fs/write"));
        assert!(rule.matches_tool("fs/delete"));
        assert!(!rule.matches_tool("other"));
    }

    #[test]
    fn join_prompt_text_concats_text_blocks() {
        let request = PromptRequest::new(
            sacp::schema::SessionId::from("sess-1"),
            vec![
                ContentBlock::from("first".to_string()),
                ContentBlock::from("second".to_string()),
            ],
        );
        let joined = ApprovalGateComponent::join_prompt_text(&request);
        assert_eq!(joined, "first second");
    }
}
