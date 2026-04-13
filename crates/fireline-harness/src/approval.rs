//! Approval gate proxy.
//!
//! Intercepts `session/prompt` requests flowing from the client
//! and runs each one through a configured policy. Matching
//! policies can either:
//!
//! - **Deny** the request outright, returning a tool-level error
//!   so the agent sees a failure instead of the requested action
//! - **RequireApproval** — emit a `permission_request` event to the
//!   durable state stream keyed by the intercepted ACP JSON-RPC
//!   `request_id`, then block the request until a matching
//!   `approval_resolved` event appears on the stream. The request is
//!   forwarded to the agent only if the resolution event carries
//!   `allow: true`. A `Deny` resolution surfaces as a gate error the
//!   agent never sees.
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

use anyhow::Result as AnyhowResult;
use durable_streams::Producer;
use fireline_acp_ids::{RequestId, SessionId, ToolCallId};
use sacp::schema::{ContentBlock, PromptRequest};
use sacp::{Agent, Client, ConnectTo, Proxy};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    CompletionKey, DurableSubscriber, DurableSubscriberDriver, PassiveSubscriber,
    PassiveWaitPolicy, StreamEnvelope,
    TraceContext,
};

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
    subscriber_driver: Arc<DurableSubscriberDriver>,
    approval_subscriber: ApprovalGateSubscriber,
    approved_sessions: Arc<Mutex<HashSet<SessionId>>>,
    pending_sessions: Arc<Mutex<HashMap<SessionId, PendingApproval>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingApproval {
    request_id: RequestId,
    reason: String,
}

#[derive(Clone)]
struct ApprovalGateSubscriber {
    approval_timeout: Option<Duration>,
}

impl ApprovalGateSubscriber {
    const NAME: &'static str = "approval_gate";

    fn new(approval_timeout: Option<Duration>) -> Self {
        Self { approval_timeout }
    }

    fn permission_request_envelope(
        &self,
        session_id: &SessionId,
        request_id: &RequestId,
        reason: &str,
    ) -> StreamEnvelope {
        permission_request_envelope(session_id.clone(), request_id.clone(), reason.to_string())
            .expect("permission request event must serialize")
    }

    fn decode_permission_event(envelope: &StreamEnvelope) -> Option<PermissionEvent> {
        (envelope.entity_type == "permission")
            .then(|| envelope.value_as::<PermissionEvent>())
            .flatten()
    }

    fn completion_event_for(
        &self,
        request_event: &PermissionEvent,
        log: &[StreamEnvelope],
    ) -> Option<PermissionEvent> {
        log.iter().find_map(|envelope| {
            let event = Self::decode_permission_event(envelope)?;
            (event.kind == "approval_resolved"
                && event.session_id == request_event.session_id
                && event.request_id == request_event.request_id)
                .then_some(event)
        })
    }
}

impl DurableSubscriber for ApprovalGateSubscriber {
    type Event = PermissionEvent;
    type Completion = PermissionEvent;

    fn name(&self) -> &str {
        Self::NAME
    }

    fn matches(&self, envelope: &StreamEnvelope) -> Option<Self::Event> {
        let event = Self::decode_permission_event(envelope)?;
        (event.kind == "permission_request" && event.request_id.is_some()).then_some(event)
    }

    fn completion_key(&self, event: &Self::Event) -> CompletionKey {
        CompletionKey::prompt(
            event.session_id.clone(),
            event
                .request_id
                .clone()
                .expect("approval permission_request must carry request_id"),
        )
    }

    fn is_completed(&self, event: &Self::Event, log: &[StreamEnvelope]) -> bool {
        self.completion_event_for(event, log).is_some()
    }
}

impl PassiveSubscriber for ApprovalGateSubscriber {
    fn wait_policy(&self) -> PassiveWaitPolicy {
        PassiveWaitPolicy {
            timeout: self.approval_timeout,
        }
    }
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
        let approval_subscriber = ApprovalGateSubscriber::new(approval_timeout);
        let mut subscriber_driver = DurableSubscriberDriver::new();
        subscriber_driver.register_passive(approval_subscriber.clone());
        Self {
            config: Arc::new(config),
            state_stream_url,
            state_producer,
            subscriber_driver: Arc::new(subscriber_driver),
            approval_subscriber,
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

    async fn rebuild_from_log(&self, session_id: &SessionId) -> Result<(), sacp::Error> {
        let Some(state_stream_url) = &self.state_stream_url else {
            return Ok(());
        };

        let log = self
            .subscriber_driver
            .replay_log(state_stream_url)
            .await
            .map_err(|error| {
                sacp::util::internal_error(format!("approval log rebuild: {error}"))
            })?;

        let mut pending = None;
        let mut approved = false;
        for envelope in &log {
            let Some(event) = ApprovalGateSubscriber::decode_permission_event(envelope) else {
                continue;
            };
            if event.session_id != *session_id {
                continue;
            }

            match event.kind.as_str() {
                "permission_request" => {
                    let Some(request_id) = event.request_id else {
                        continue;
                    };
                    let Some(reason) = event.reason else {
                        continue;
                    };
                    approved = false;
                    pending = Some(PendingApproval { request_id, reason });
                }
                "approval_resolved" => {
                    pending = None;
                    approved = event.allow.unwrap_or(false);
                }
                _ => {}
            }
        }

        if approved {
            self.approved_sessions
                .lock()
                .expect("approval state poisoned")
                .insert(session_id.clone());
            self.pending_sessions
                .lock()
                .expect("approval state poisoned")
                .remove(session_id);
        } else if let Some(pending) = pending {
            self.pending_sessions
                .lock()
                .expect("approval state poisoned")
                .insert(session_id.clone(), pending);
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
        session_id: &SessionId,
        request_id: &RequestId,
        reason: &str,
    ) -> Result<(), sacp::Error> {
        let approval_emit_span = tracing::info_span!(
            "fireline.approval.emit",
            session_id = %session_id,
            request_id = %request_id,
        );
        let _approval_emit_guard = approval_emit_span.enter();

        let Some(producer) = self.state_producer.as_ref() else {
            return Err(sacp::util::internal_error(
                "approval gate has no state producer; cannot emit permission_request",
            ));
        };

        producer.append_json(
            &self
                .approval_subscriber
                .permission_request_envelope(session_id, request_id, reason),
        );
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
        session_id: &SessionId,
        request_id: &RequestId,
    ) -> Result<bool, sacp::Error> {
        let approval_wait_span = tracing::info_span!(
            "fireline.approval.wait",
            session_id = %session_id,
            request_id = %request_id,
        );
        let _approval_wait_guard = approval_wait_span.enter();

        let state_stream_url = self
            .state_stream_url
            .as_deref()
            .ok_or_else(|| sacp::util::internal_error("approval gate has no state stream URL"))?;

        let request_envelope = self
            .approval_subscriber
            .permission_request_envelope(session_id, request_id, "pending");
        let request_event = self
            .approval_subscriber
            .matches(&request_envelope)
            .expect("approval request envelope must match the approval subscriber");
        let log = self
            .subscriber_driver
            .wait_for_passive_completion(
                self.approval_subscriber.name(),
                &request_envelope,
                state_stream_url,
            )
            .await
            .map_err(|error| map_wait_error(session_id, error))?;

        let resolution = self
            .approval_subscriber
            .completion_event_for(&request_event, &log)
            .ok_or_else(|| {
                sacp::util::internal_error(
                    "approval stream closed before the permission was resolved",
                )
            })?;

        let allow = resolution.allow.unwrap_or(false);
        let resolved_by = resolution.resolved_by.as_deref().unwrap_or("unknown");
        tracing::info_span!(
            "fireline.approval.resolve",
            session_id = %session_id,
            request_id = %request_id,
            allow,
            resolved_by = %resolved_by,
        )
        .in_scope(|| {});
        Ok(allow)
    }

    fn is_session_approved(&self, session_id: &SessionId) -> bool {
        self.approved_sessions
            .lock()
            .expect("approval state poisoned")
            .contains(session_id)
    }
}

fn approval_timeout_error(session_id: &SessionId) -> sacp::Error {
    sacp::util::internal_error(format!(
        "approval_gate timed out waiting for approval on session {session_id}"
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PermissionEvent {
    kind: String,
    session_id: SessionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<RequestId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<ToolCallId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    allow: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    created_at_ms: i64,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    meta: Option<TraceContext>,
}

pub fn permission_request_envelope(
    session_id: SessionId,
    request_id: RequestId,
    reason: String,
) -> AnyhowResult<StreamEnvelope> {
    Ok(StreamEnvelope {
        entity_type: "permission".to_string(),
        key: format!("{session_id}:{}", request_id_key(&request_id)),
        headers: insert_headers(),
        source_offset: None,
        value: Some(serde_json::to_value(PermissionEvent {
            kind: "permission_request".to_string(),
            session_id,
            request_id: Some(request_id),
            tool_call_id: None,
            allow: None,
            resolved_by: None,
            reason: Some(reason),
            created_at_ms: now_ms(),
            meta: None,
        })?),
    })
}

pub fn approval_resolution_envelope(
    session_id: SessionId,
    request_id: RequestId,
    allow: bool,
    resolved_by: String,
) -> AnyhowResult<StreamEnvelope> {
    approval_resolution_envelope_with_trace(session_id, request_id, allow, resolved_by, None)
}

pub fn approval_resolution_envelope_with_trace(
    session_id: SessionId,
    request_id: RequestId,
    allow: bool,
    resolved_by: String,
    meta: Option<TraceContext>,
) -> AnyhowResult<StreamEnvelope> {
    Ok(StreamEnvelope {
        entity_type: "permission".to_string(),
        key: format!("{session_id}:{}:resolved", request_id_key(&request_id)),
        headers: insert_headers(),
        source_offset: None,
        value: Some(serde_json::to_value(PermissionEvent {
            kind: "approval_resolved".to_string(),
            session_id,
            request_id: Some(request_id),
            tool_call_id: None,
            allow: Some(allow),
            resolved_by: Some(resolved_by),
            reason: None,
            created_at_ms: now_ms(),
            meta,
        })?),
    })
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
                        let session_id = request.session_id.clone();
                        let prompt_text = ApprovalGateComponent::join_prompt_text(&request);
                        if this.is_session_approved(&session_id) {
                            return cx
                                .send_request_to(Agent, request)
                                .forward_response_to(responder);
                        }

                        let policy_match = this.config.policies.iter().find_map(|policy| {
                            policy
                                .match_rule
                                .matches_prompt(&prompt_text)
                                .then(|| (policy.action, policy.reason.clone()))
                        });

                        let Some((action, reason)) = policy_match else {
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
                                let request_id_value = responder.id();
                                let Some(request_id) = request_id_from_value(&request_id_value) else {
                                    return responder.respond_with_error(sacp::util::internal_error(
                                        format!(
                                            "approval_gate requires canonical JSON-RPC request id; got {request_id_value}"
                                        ),
                                    ));
                                };
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
                        this.rebuild_from_log(&request.session_id).await?;
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

fn insert_headers() -> Map<String, Value> {
    let mut headers = Map::new();
    headers.insert("operation".to_string(), Value::String("insert".to_string()));
    headers
}

fn map_wait_error(session_id: &SessionId, error: anyhow::Error) -> sacp::Error {
    let error_text = error.to_string();
    if error_text.contains("timed out waiting for passive durable subscriber completion") {
        approval_timeout_error(session_id)
    } else if error_text.contains("stream closed before completion") {
        sacp::util::internal_error("approval stream closed before the permission was resolved")
    } else {
        sacp::util::internal_error(format!("approval stream read error: {error_text}"))
    }
}

fn request_id_from_value(value: &Value) -> Option<RequestId> {
    serde_json::from_value(value.clone()).ok()
}

fn request_id_key(request_id: &RequestId) -> String {
    match request_id {
        RequestId::Null => "null".to_string(),
        RequestId::Number(number) => number.to_string(),
        RequestId::Str(text) => text.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Context, Result};
    use axum::Router;
    use durable_streams::{Client as DurableStreamsClient, CreateOptions};
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
        let session_id = SessionId::from(session_id.to_string());
        let request_id = RequestId::from(request_id.to_string());
        producer.append_json(&approval_resolution_envelope(
            session_id,
            request_id,
            allow,
            "approval-test".to_string(),
        )?);
        producer
            .flush()
            .await
            .context("flush approval_resolved test event")?;
        Ok(())
    }

    async fn seed_permission_stream(producer: &Producer) -> Result<()> {
        producer.append_json(&StreamEnvelope {
            entity_type: "permission".to_string(),
            key: "bootstrap".to_string(),
            headers: insert_headers(),
            source_offset: None,
            value: Some(serde_json::to_value(PermissionEvent {
                kind: "permission_request".to_string(),
                session_id: SessionId::from("bootstrap-session"),
                request_id: Some(RequestId::from("bootstrap-request".to_string())),
                tool_call_id: None,
                allow: None,
                resolved_by: None,
                reason: Some("bootstrap".to_string()),
                created_at_ms: now_ms(),
                meta: None,
            })?),
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
    fn request_id_from_value_accepts_canonical_string_and_number_ids() {
        assert_eq!(
            request_id_from_value(&Value::String("req-123".to_string())),
            Some(RequestId::from("req-123".to_string()))
        );
        assert_eq!(
            request_id_from_value(&Value::Number(serde_json::Number::from(42))),
            Some(serde_json::from_value(serde_json::json!(42)).expect("numeric request id"))
        );
    }

    #[test]
    fn request_id_from_value_rejects_non_json_rpc_ids() {
        assert_eq!(
            request_id_from_value(&serde_json::json!({"bad": "id"})),
            None
        );
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
            async move {
                let session_id = SessionId::from("session-a");
                let request_id = RequestId::from("request-a".to_string());
                gate.wait_for_approval(&session_id, &request_id).await
            }
        });
        let mut waiter_b = tokio::spawn({
            let gate = gate.clone();
            async move {
                let session_id = SessionId::from("session-b");
                let request_id = RequestId::from("request-b".to_string());
                gate.wait_for_approval(&session_id, &request_id).await
            }
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
