//! Approval gate proxy — SKETCH.
//!
//! Intended shape: intercept outbound tool calls matching a policy,
//! issue `session/request_permission` back toward the client, forward
//! on approve or return a tool error on deny.
//!
//! # SKETCH STATUS
//!
//! - Config, policy, matcher, and `ApprovalConfig::policy_for` lookup
//!   are fully implemented and tested.
//! - The `ConnectTo<Conductor>` impl is a pass-through proxy. The
//!   actual outbound tool-call interception + `session/request_permission`
//!   round-trip is TODO because tool calls travel as MCP-over-ACP and
//!   don't present a clean typed ACP request shape to intercept at
//!   the proxy level. See the SDK gap flagged in
//!   `docs/programmable-topology-exploration.md`.

use sacp::{ConnectTo, Proxy};

#[derive(Clone, Default)]
pub struct ApprovalConfig {
    pub policies: Vec<ApprovalPolicy>,
}

#[derive(Clone)]
pub struct ApprovalPolicy {
    pub match_rule: ApprovalMatch,
    pub action: ApprovalAction,
}

#[derive(Clone)]
pub enum ApprovalMatch {
    /// Exact match by tool name.
    Tool { name: String },
    /// Prefix match on the tool name (e.g., `"fs/"` matches `"fs/write"`).
    ToolPrefix { prefix: String },
    // TODO: add parameter-matching and regex variants once real
    // policies are being written.
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalAction {
    /// Pause the call and ask the user via `session/request_permission`.
    RequireApproval,
    /// Refuse the call with a tool error. The agent sees a failure.
    Deny,
}

impl ApprovalMatch {
    pub fn matches_tool(&self, tool_name: &str) -> bool {
        match self {
            ApprovalMatch::Tool { name } => name == tool_name,
            ApprovalMatch::ToolPrefix { prefix } => tool_name.starts_with(prefix.as_str()),
        }
    }
}

impl ApprovalConfig {
    /// Return the first matching policy for a given tool name, or
    /// `None` if no policy applies (implicit allow).
    pub fn policy_for(&self, tool_name: &str) -> Option<&ApprovalPolicy> {
        self.policies
            .iter()
            .find(|p| p.match_rule.matches_tool(tool_name))
    }
}

#[derive(Clone)]
pub struct ApprovalGateComponent {
    config: ApprovalConfig,
}

impl ApprovalGateComponent {
    pub fn new(config: ApprovalConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &ApprovalConfig {
        &self.config
    }
}

impl ConnectTo<sacp::Conductor> for ApprovalGateComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        let _config = self.config;
        // TODO: intercept outbound tool calls. On each call:
        //   1. Look up `_config.policy_for(tool_name)`
        //   2. If `Some(policy)`:
        //      - `RequireApproval`: issue `session/request_permission`
        //        back to the client, await answer, forward or error.
        //      - `Deny`: return a tool error immediately.
        //   3. If `None`: forward transparently (implicit allow).
        sacp::Proxy
            .builder()
            .name("fireline-approval")
            .connect_to(client)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_tool_match() {
        let rule = ApprovalMatch::Tool {
            name: "shell".to_string(),
        };
        assert!(rule.matches_tool("shell"));
        assert!(!rule.matches_tool("shell_run"));
        assert!(!rule.matches_tool("read_file"));
    }

    #[test]
    fn prefix_tool_match() {
        let rule = ApprovalMatch::ToolPrefix {
            prefix: "fs/".to_string(),
        };
        assert!(rule.matches_tool("fs/write"));
        assert!(rule.matches_tool("fs/delete"));
        assert!(!rule.matches_tool("fs"));
        assert!(!rule.matches_tool("other"));
    }

    #[test]
    fn policy_lookup_returns_first_match() {
        let config = ApprovalConfig {
            policies: vec![
                ApprovalPolicy {
                    match_rule: ApprovalMatch::Tool {
                        name: "shell".to_string(),
                    },
                    action: ApprovalAction::RequireApproval,
                },
                ApprovalPolicy {
                    match_rule: ApprovalMatch::ToolPrefix {
                        prefix: "secret/".to_string(),
                    },
                    action: ApprovalAction::Deny,
                },
            ],
        };

        assert_eq!(
            config.policy_for("shell").unwrap().action,
            ApprovalAction::RequireApproval
        );
        assert_eq!(
            config.policy_for("secret/api_key").unwrap().action,
            ApprovalAction::Deny
        );
        assert!(config.policy_for("read_file").is_none());
    }
}
