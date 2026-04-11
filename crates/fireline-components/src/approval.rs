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

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use durable_streams::{Client as DurableStreamsClient, Producer};
use sacp::schema::{ContentBlock, PromptRequest};
use sacp::{Agent, Client, ConnectTo, Proxy};
use serde::Serialize;
use serde_json::Value;

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
            ApprovalMatch::PromptContains { needle } => prompt_text
                .to_ascii_lowercase()
                .contains(&needle.to_ascii_lowercase()),
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
    state_stream_url: Option<String>,
    state_producer: Option<Producer>,
    approved_sessions: Arc<Mutex<HashSet<String>>>,
    pending_sessions: Arc<Mutex<HashMap<String, String>>>,
}

impl ApprovalGateComponent {
    pub fn new(config: ApprovalConfig) -> Self {
        Self::with_stream(config, None, None)
    }

    pub fn with_stream(
        config: ApprovalConfig,
        state_stream_url: Option<String>,
        state_producer: Option<Producer>,
    ) -> Self {
        Self {
            config: Arc::new(config),
            state_stream_url,
            state_producer,
            approved_sessions: Arc::new(Mutex::new(HashSet::new())),
            pending_sessions: Arc::new(Mutex::new(HashMap::new())),
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

    async fn rebuild_from_log(&self, session_id: &str) -> Result<(), sacp::Error> {
        let Some(state_stream_url) = &self.state_stream_url else {
            return Ok(());
        };

        let client = DurableStreamsClient::new();
        let stream = client.stream(state_stream_url);
        let mut reader = stream
            .read()
            .offset(durable_streams::Offset::Beginning)
            .build()
            .map_err(|error| sacp::util::internal_error(format!("approval log rebuild: {error}")))?;

        let mut pending_reason = None;
        let mut approved = false;
        while let Some(chunk) = reader
            .next_chunk()
            .await
            .map_err(|error| sacp::util::internal_error(format!("approval log rebuild: {error}")))?
        {
            if chunk.data.is_empty() {
                if chunk.up_to_date {
                    break;
                }
                continue;
            }

            let events: Vec<Value> = serde_json::from_slice(&chunk.data)
                .map_err(|error| sacp::util::internal_error(format!("approval log parse: {error}")))?;
            for event in events {
                let Some(value) = event.get("value") else {
                    continue;
                };
                if value.get("sessionId").and_then(Value::as_str) != Some(session_id) {
                    continue;
                }

                match value.get("kind").and_then(Value::as_str) {
                    Some("permission_request") => {
                        pending_reason = value
                            .get("reason")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                    }
                    Some("approval_resolved") => {
                        approved = value.get("allow").and_then(Value::as_bool).unwrap_or(false);
                    }
                    _ => {}
                }
            }

            if chunk.up_to_date {
                break;
            }
        }

        if approved {
            self.approved_sessions
                .lock()
                .expect("approval state poisoned")
                .insert(session_id.to_string());
            self.pending_sessions
                .lock()
                .expect("approval state poisoned")
                .remove(session_id);
        } else if let Some(reason) = pending_reason {
            self.pending_sessions
                .lock()
                .expect("approval state poisoned")
                .insert(session_id.to_string(), reason);
        }

        Ok(())
    }

    fn emit_permission_request(&self, session_id: &str, reason: &str) {
        let Some(producer) = self.state_producer.as_ref() else {
            return;
        };

        producer.append_json(&StateEnvelope {
            entity_type: "permission",
            key: format!("{session_id}:{}", now_ms()),
            headers: StateHeaders {
                operation: "insert",
            },
            value: PermissionEvent {
                kind: "permission_request",
                session_id: session_id.to_string(),
                allow: None,
                resolved_by: None,
                reason: Some(reason.to_string()),
                created_at_ms: now_ms(),
            },
        });
    }

    fn is_session_approved(&self, session_id: &str) -> bool {
        self.approved_sessions
            .lock()
            .expect("approval state poisoned")
            .contains(session_id)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionEvent {
    kind: &'static str,
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    allow: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
struct StateHeaders {
    operation: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct StateEnvelope<T> {
    #[serde(rename = "type")]
    entity_type: &'static str,
    key: String,
    headers: StateHeaders,
    value: T,
}

impl ConnectTo<sacp::Conductor> for ApprovalGateComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        let this = self.clone();
        sacp::Proxy
            .builder()
            .name("fireline-approval")
            .on_receive_request_from(
                Client,
                {
                    let this = this.clone();
                    async move |request: PromptRequest, responder, cx| {
                        let session_id = request.session_id.to_string();
                        let prompt_text = ApprovalGateComponent::join_prompt_text(&request);
                        if this.is_session_approved(&session_id) {
                            return cx
                                .send_request_to(Agent, request)
                                .forward_response_to(responder);
                        }

                        if let Some(policy) = this.config.policy_for_prompt(&prompt_text) {
                            match policy.action {
                                ApprovalAction::Deny => {
                                    return Err(sacp::util::internal_error(format!(
                                        "approval_gate denied prompt: {}",
                                        policy.reason
                                    )));
                                }
                                ApprovalAction::RequireApproval => {
                                    let result = cx
                                        .send_request_to(Agent, request)
                                        .forward_response_to(responder);
                                    this.emit_permission_request(&session_id, &policy.reason);
                                    this.pending_sessions
                                        .lock()
                                        .expect("approval state poisoned")
                                        .insert(session_id, policy.reason.clone());
                                    return result;
                                }
                            }
                        }
                        cx.send_request_to(Agent, request)
                            .forward_response_to(responder)
                    }
                },
                sacp::on_receive_request!(),
            )
            .on_receive_request_from(
                Client,
                {
                    let this = this.clone();
                    async move |request: sacp::schema::LoadSessionRequest, responder, cx| {
                        this.rebuild_from_log(&request.session_id.to_string()).await?;
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

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
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
        let policy = config
            .policy_for_prompt("help me write a DROP TABLE query")
            .unwrap();
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
