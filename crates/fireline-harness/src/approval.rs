//! Approval gate proxy.
//!
//! Intercepts `session/prompt` requests flowing from the client
//! and runs each one through a configured policy. Matching
//! policies can either:
//!
//! - **Deny** the request outright, returning a tool-level error
//!   so the agent sees a failure instead of the requested action
//! - **RequireApproval** — emit a `permission_request` event to the
//!   durable state stream with a unique request id, then block the
//!   request until a matching `approval_resolved` event appears on
//!   the stream. The request is forwarded to the agent only if the
//!   resolution event carries `allow: true`. A `Deny` resolution
//!   surfaces as a gate error the agent never sees.
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
use std::time::Duration;

use durable_streams::{Client as DurableStreamsClient, LiveMode, Offset, Producer};
use sacp::schema::{ContentBlock, PromptRequest};
use sacp::{Agent, Client, ConnectTo, Proxy};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

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
    /// Emit a `permission_request` event and block the request
    /// until a matching `approval_resolved` event appears on the
    /// durable state stream. The downstream agent only sees the
    /// request if the resolution event is `allow: true`.
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
    approval_timeout: Option<Duration>,
    approved_sessions: Arc<Mutex<HashSet<String>>>,
    pending_sessions: Arc<Mutex<HashMap<String, PendingApproval>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingApproval {
    request_id: String,
    reason: String,
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
        Self::with_stream_and_timeout(config, state_stream_url, state_producer, None)
    }

    pub fn with_stream_and_timeout(
        config: ApprovalConfig,
        state_stream_url: Option<String>,
        state_producer: Option<Producer>,
        approval_timeout: Option<Duration>,
    ) -> Self {
        Self {
            config: Arc::new(config),
            state_stream_url,
            state_producer,
            approval_timeout,
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

    // The approval gate runs before the state projector materializes a
    // prompt_turn row, so the final prompt_turn_id is not directly available
    // here. Prefer a caller-supplied Fireline trace id when present because
    // it becomes the downstream prompt_turn_id; otherwise fall back to a
    // stable serialization of the prompt payload so replay of the same prompt
    // in the same session resolves to the same approval request id.
    fn prompt_identity(request: &PromptRequest) -> String {
        request
            .meta
            .as_ref()
            .and_then(fireline_trace_id)
            .unwrap_or_else(|| {
                serde_json::to_string(&request.prompt)
                    .unwrap_or_else(|_| ApprovalGateComponent::join_prompt_text(request))
            })
    }

    fn approval_request_id(request: &PromptRequest, policy_id: usize) -> String {
        let prompt_identity = Self::prompt_identity(request);
        let material = format!("{}:{policy_id}:{prompt_identity}", request.session_id);
        let digest = Sha256::digest(material.as_bytes());
        format!("{digest:x}")
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
            .map_err(|error| {
                sacp::util::internal_error(format!("approval log rebuild: {error}"))
            })?;

        let mut pending = None;
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

            let events: Vec<Value> = serde_json::from_slice(&chunk.data).map_err(|error| {
                sacp::util::internal_error(format!("approval log parse: {error}"))
            })?;
            for event in events {
                let Some(value) = event.get("value") else {
                    continue;
                };
                if value.get("sessionId").and_then(Value::as_str) != Some(session_id) {
                    continue;
                }

                match value.get("kind").and_then(Value::as_str) {
                    Some("permission_request") => {
                        let Some(request_id) = value
                            .get("requestId")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                        else {
                            continue;
                        };
                        let Some(reason) = value
                            .get("reason")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                        else {
                            continue;
                        };
                        approved = false;
                        pending = Some(PendingApproval { request_id, reason });
                    }
                    Some("approval_resolved") => {
                        pending = None;
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
        } else if let Some(pending) = pending {
            self.pending_sessions
                .lock()
                .expect("approval state poisoned")
                .insert(session_id.to_string(), pending);
        } else {
            self.pending_sessions
                .lock()
                .expect("approval state poisoned")
                .remove(session_id);
        }

        Ok(())
    }

    async fn emit_permission_request(
        &self,
        session_id: &str,
        request_id: &str,
        reason: &str,
    ) -> Result<(), sacp::Error> {
        let Some(producer) = self.state_producer.as_ref() else {
            return Err(sacp::util::internal_error(
                "approval gate has no state producer; cannot emit permission_request",
            ));
        };

        producer.append_json(&StateEnvelope {
            entity_type: "permission",
            key: format!("{session_id}:{request_id}"),
            headers: StateHeaders {
                operation: "insert",
            },
            value: PermissionEvent {
                kind: "permission_request",
                session_id: session_id.to_string(),
                request_id: Some(request_id.to_string()),
                allow: None,
                resolved_by: None,
                reason: Some(reason.to_string()),
                created_at_ms: now_ms(),
            },
        });
        producer
            .flush()
            .await
            .map_err(|error| sacp::util::internal_error(format!("approval flush: {error}")))?;
        Ok(())
    }

    /// Block until an `approval_resolved` event with a matching
    /// `request_id` appears on the state stream. Returns `Ok(true)`
    /// if the request was approved, `Ok(false)` if explicitly
    /// denied, or an error if the gate timeout elapses or the
    /// stream terminates.
    async fn wait_for_approval(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Result<bool, sacp::Error> {
        let state_stream_url = self
            .state_stream_url
            .as_deref()
            .ok_or_else(|| sacp::util::internal_error("approval gate has no state stream URL"))?;

        let client = DurableStreamsClient::new();
        let stream = client.stream(state_stream_url);
        let mut reader = stream
            .read()
            .offset(Offset::Beginning)
            .live(LiveMode::Sse)
            .build()
            .map_err(|error| {
                sacp::util::internal_error(format!("approval stream reader: {error}"))
            })?;

        let deadline = self
            .approval_timeout
            .map(|timeout| tokio::time::Instant::now() + timeout);

        loop {
            let next = if let Some(deadline) = deadline {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    return Err(approval_timeout_error(session_id));
                }
                match tokio::time::timeout(remaining, reader.next_chunk()).await {
                    Ok(chunk_result) => chunk_result,
                    Err(_) => return Err(approval_timeout_error(session_id)),
                }
            } else {
                reader.next_chunk().await
            };

            match next {
                Ok(Some(chunk)) => {
                    if chunk.data.is_empty() {
                        continue;
                    }
                    let events: Vec<Value> =
                        serde_json::from_slice(&chunk.data).map_err(|error| {
                            sacp::util::internal_error(format!("approval parse: {error}"))
                        })?;
                    for event in events {
                        let Some(value) = event.get("value") else {
                            continue;
                        };
                        if value.get("kind").and_then(Value::as_str) != Some("approval_resolved") {
                            continue;
                        }
                        if value.get("sessionId").and_then(Value::as_str) != Some(session_id) {
                            continue;
                        }
                        if value.get("requestId").and_then(Value::as_str) != Some(request_id) {
                            continue;
                        }
                        let allow = value.get("allow").and_then(Value::as_bool).unwrap_or(false);
                        return Ok(allow);
                    }
                }
                Ok(None) => {
                    return Err(sacp::util::internal_error(
                        "approval stream closed before the permission was resolved",
                    ));
                }
                Err(error) => {
                    return Err(sacp::util::internal_error(format!(
                        "approval stream read error: {error}"
                    )));
                }
            }
        }
    }

    fn is_session_approved(&self, session_id: &str) -> bool {
        self.approved_sessions
            .lock()
            .expect("approval state poisoned")
            .contains(session_id)
    }
}

fn approval_timeout_error(session_id: &str) -> sacp::Error {
    sacp::util::internal_error(format!(
        "approval_gate timed out waiting for approval on session {session_id}"
    ))
}

fn fireline_trace_id(meta: &serde_json::Map<String, Value>) -> Option<String> {
    meta.get("fireline")
        .and_then(Value::as_object)
        .and_then(|fireline| fireline.get("traceId"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            meta.get("fireline/trace-id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionEvent {
    kind: &'static str,
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
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

                        let policy_match =
                            this.config.policies.iter().enumerate().find_map(|(policy_id, policy)| {
                                policy
                                    .match_rule
                                    .matches_prompt(&prompt_text)
                                    .then(|| (policy_id, policy.action, policy.reason.clone()))
                            });

                        let Some((policy_id, action, reason)) = policy_match else {
                            return cx
                                .send_request_to(Agent, request)
                                .forward_response_to(responder);
                        };

                        match action {
                            ApprovalAction::Deny => responder.respond_with_error(
                                sacp::util::internal_error(format!(
                                    "approval_gate denied prompt: {reason}"
                                )),
                            ),
                            ApprovalAction::RequireApproval => {
                                let request_id =
                                    ApprovalGateComponent::approval_request_id(&request, policy_id);
                                let should_emit = {
                                    let mut pending_sessions = this
                                        .pending_sessions
                                        .lock()
                                        .expect("approval state poisoned");
                                    let should_emit = pending_sessions
                                        .get(&session_id)
                                        .map(|pending| pending.request_id != request_id)
                                        .unwrap_or(true);
                                    pending_sessions.insert(
                                        session_id.clone(),
                                        PendingApproval {
                                            request_id: request_id.clone(),
                                            reason: reason.clone(),
                                        },
                                    );
                                    should_emit
                                };
                                if should_emit {
                                    if let Err(error) =
                                        this.emit_permission_request(&session_id, &request_id, &reason)
                                            .await
                                    {
                                        this.pending_sessions
                                            .lock()
                                            .expect("approval state poisoned")
                                            .remove(&session_id);
                                        return responder.respond_with_error(error);
                                    }
                                }
                                let allowed =
                                    match this.wait_for_approval(&session_id, &request_id).await {
                                        Ok(allowed) => allowed,
                                        Err(error) => {
                                            this.pending_sessions
                                                .lock()
                                                .expect("approval state poisoned")
                                                .remove(&session_id);
                                            return responder.respond_with_error(error);
                                        }
                                    };
                                this.pending_sessions
                                    .lock()
                                    .expect("approval state poisoned")
                                    .remove(&session_id);
                                if !allowed {
                                    return responder.respond_with_error(sacp::util::internal_error(
                                        format!("approval_gate denied by approver: {reason}"),
                                    ));
                                }
                                this.approved_sessions
                                    .lock()
                                    .expect("approval state poisoned")
                                    .insert(session_id);
                                cx.send_request_to(Agent, request)
                                    .forward_response_to(responder)
                            }
                        }
                    }
                },
                sacp::on_receive_request!(),
            )
            .on_receive_request_from(
                Client,
                {
                    let this = this.clone();
                    async move |request: sacp::schema::LoadSessionRequest, responder, cx| {
                        this.rebuild_from_log(&request.session_id.to_string())
                            .await?;
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
    use anyhow::{Context, Result};
    use axum::Router;
    use super::*;
    use durable_streams::CreateOptions;
    use tokio::sync::oneshot;

    struct TestStreamServer {
        base_url: String,
        shutdown_tx: Option<oneshot::Sender<()>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl TestStreamServer {
        async fn spawn() -> Result<Self> {
            let router: Router = fireline_session::build_stream_router(None)?;
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .context("bind durable-streams test listener")?;
            let addr = listener
                .local_addr()
                .context("resolve durable-streams test listener")?;
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            let task = tokio::spawn(async move {
                let _ = axum::serve(listener, router)
                    .with_graceful_shutdown(async {
                        let _ = shutdown_rx.await;
                    })
                    .await;
            });
            Ok(Self {
                base_url: format!("http://127.0.0.1:{}/v1/stream", addr.port()),
                shutdown_tx: Some(shutdown_tx),
                task,
            })
        }

        fn stream_url(&self, stream_name: &str) -> String {
            format!("{}/{}", self.base_url.trim_end_matches('/'), stream_name)
        }

        async fn shutdown(mut self) {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(());
            }
            let _ = self.task.await;
        }
    }

    fn test_permission_producer(stream_url: &str) -> Producer {
        let client = DurableStreamsClient::new();
        let mut stream = client.stream(stream_url);
        stream.set_content_type("application/json");
        stream
            .producer(format!("approval-test-writer-{}", uuid::Uuid::new_v4()))
            .content_type("application/json")
            .build()
    }

    async fn ensure_json_stream_exists(stream_url: &str) -> Result<()> {
        let client = DurableStreamsClient::new();
        let stream = client.stream(stream_url);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match stream
                .create_with(CreateOptions::new().content_type("application/json"))
                .await
            {
                Ok(_) | Err(durable_streams::StreamError::Conflict) => return Ok(()),
                Err(error) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    tracing::debug!(?error, stream_url, "retrying approval test stream creation");
                }
                Err(error) => {
                    return Err(anyhow::Error::from(error))
                        .with_context(|| format!("create approval test stream '{stream_url}'"));
                }
            }
        }
    }

    async fn append_approval_resolved_event(
        producer: &Producer,
        session_id: &str,
        request_id: &str,
        allow: bool,
    ) -> Result<()> {
        producer.append_json(&StateEnvelope {
            entity_type: "permission",
            key: format!("{session_id}:{request_id}:resolved"),
            headers: StateHeaders {
                operation: "insert",
            },
            value: PermissionEvent {
                kind: "approval_resolved",
                session_id: session_id.to_string(),
                request_id: Some(request_id.to_string()),
                allow: Some(allow),
                resolved_by: Some("approval-test".to_string()),
                reason: None,
                created_at_ms: now_ms(),
            },
        });
        producer
            .flush()
            .await
            .context("flush approval_resolved test event")?;
        Ok(())
    }

    async fn seed_permission_stream(producer: &Producer) -> Result<()> {
        producer.append_json(&StateEnvelope {
            entity_type: "permission",
            key: "bootstrap".to_string(),
            headers: StateHeaders {
                operation: "insert",
            },
            value: PermissionEvent {
                kind: "permission_request",
                session_id: "bootstrap-session".to_string(),
                request_id: Some("bootstrap-request".to_string()),
                allow: None,
                resolved_by: None,
                reason: Some("bootstrap".to_string()),
                created_at_ms: now_ms(),
            },
        });
        producer
            .flush()
            .await
            .context("flush approval bootstrap event")?;
        Ok(())
    }

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

    #[test]
    fn approval_request_id_uses_fireline_trace_id_when_present() {
        let mut fireline = serde_json::Map::new();
        fireline.insert("traceId".to_string(), Value::String("trace-123".to_string()));
        let mut meta = serde_json::Map::new();
        meta.insert("fireline".to_string(), Value::Object(fireline));

        let request = PromptRequest::new(
            sacp::schema::SessionId::from("sess-1"),
            vec![ContentBlock::from("same prompt".to_string())],
        )
        .meta(meta);

        let first = ApprovalGateComponent::approval_request_id(&request, 0);
        let second = ApprovalGateComponent::approval_request_id(&request, 0);
        assert_eq!(first, second);
    }

    #[test]
    fn approval_request_id_changes_with_policy_id() {
        let request = PromptRequest::new(
            sacp::schema::SessionId::from("sess-1"),
            vec![ContentBlock::from("same prompt".to_string())],
        );

        let first = ApprovalGateComponent::approval_request_id(&request, 0);
        let second = ApprovalGateComponent::approval_request_id(&request, 1);
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn concurrent_waiters_are_isolated_by_session_and_request_id() -> Result<()> {
        let server = TestStreamServer::spawn().await?;
        let stream_url = server.stream_url(&format!("approval-gate-{}", uuid::Uuid::new_v4()));
        ensure_json_stream_exists(&stream_url).await?;
        let producer = test_permission_producer(&stream_url);
        seed_permission_stream(&producer).await?;
        let gate = ApprovalGateComponent::with_stream_and_timeout(
            ApprovalConfig::default(),
            Some(stream_url),
            None,
            Some(Duration::from_secs(5)),
        );

        let waiter_a = tokio::spawn({
            let gate = gate.clone();
            async move { gate.wait_for_approval("session-a", "request-a").await }
        });
        let mut waiter_b = tokio::spawn({
            let gate = gate.clone();
            async move { gate.wait_for_approval("session-b", "request-b").await }
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        append_approval_resolved_event(&producer, "session-a", "request-a", true).await?;

        let allow_a = tokio::time::timeout(Duration::from_secs(2), waiter_a)
            .await
            .context("session A waiter did not resolve after matching approval")?
            .context("session A waiter panicked")??;
        assert!(
            allow_a,
            "INVARIANT (ApprovalGate): matching approval_resolved must release the corresponding waiter"
        );

        assert!(
            tokio::time::timeout(Duration::from_millis(250), &mut waiter_b)
                .await
                .is_err(),
            "INVARIANT (ApprovalGate): resolving session A must not release session B's waiter"
        );

        append_approval_resolved_event(&producer, "session-b", "request-b", true).await?;
        let allow_b = tokio::time::timeout(Duration::from_secs(2), waiter_b)
            .await
            .context("session B waiter did not resolve after its own approval")?
            .context("session B waiter panicked")??;
        assert!(
            allow_b,
            "INVARIANT (ApprovalGate): session B must release once its own approval_resolved arrives"
        );

        server.shutdown().await;
        Ok(())
    }
}
