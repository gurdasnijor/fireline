//! Approval gate proxy.
//!
//! Supports two gate points:
//!
//! - Prompt-scoped approval via `session/prompt` interception when a
//!   configured prompt policy matches
//! - Tool-call-scoped approval via ACP `session/request_permission`
//!   interception when a configured tool policy matches
//!
//! Matching policies can either:
//!
//! - **Deny** the request outright
//! - **RequireApproval** — emit a `permission_request` event to the
//!   durable state stream keyed by canonical ACP identity, then block
//!   until a matching `approval_resolved` event appears on the stream
//!
//! Prompt approvals key on `(session_id, request_id)`. Tool approvals
//! key on `(session_id, tool_call_id)` so replay and rebuild follow the
//! same canonical completion spine as other tool-scoped subscribers.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result as AnyhowResult;
use durable_streams::Producer;
use fireline_acp_ids::{RequestId, SessionId, ToolCallId};
use sacp::schema::{
    ContentBlock, PermissionOption, PermissionOptionKind, PromptRequest, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome,
};
use sacp::{Agent, Client, ConnectTo, Proxy};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    CompletionKey, DurableSubscriber, DurableSubscriberDriver, PassiveSubscriber,
    PassiveWaitPolicy, StreamEnvelope, TraceContext,
};

use crate::agent_observability::{
    clear_approval_requested_span, emit_approval_resolved_span, ensure_prompt_request_span,
    start_approval_requested_span,
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
    /// Exact match by tool-call title on ACP `session/request_permission`.
    Tool { name: String },
    /// Prefix match on the tool-call title.
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

impl ApprovalPolicy {
    pub fn policy_id(&self) -> String {
        match &self.match_rule {
            ApprovalMatch::PromptContains { needle } => format!("prompt_contains:{needle}"),
            ApprovalMatch::Tool { name } => format!("tool:{name}"),
            ApprovalMatch::ToolPrefix { prefix } => format!("tool_prefix:{prefix}"),
        }
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
    resolved_tool_calls: Arc<Mutex<HashMap<(SessionId, ToolCallId), ApprovalResolution>>>,
    pending_tool_calls: Arc<Mutex<HashMap<(SessionId, ToolCallId), String>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingApproval {
    request_id: RequestId,
    reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ApprovalSubject {
    Prompt {
        session_id: SessionId,
        request_id: RequestId,
    },
    ToolCall {
        session_id: SessionId,
        tool_call_id: ToolCallId,
    },
}

impl ApprovalSubject {
    fn stream_key(&self) -> String {
        match self {
            Self::Prompt {
                session_id,
                request_id,
            } => format!("{session_id}:{}", request_id_key(request_id)),
            Self::ToolCall {
                session_id,
                tool_call_id,
            } => format!("{session_id}:{tool_call_id}"),
        }
    }

    fn resolved_stream_key(&self) -> String {
        format!("{}:resolved", self.stream_key())
    }
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

    fn permission_request_envelope(&self, event: &PermissionEvent) -> StreamEnvelope {
        permission_request_envelope_from_event(event.clone())
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
        let expected_subject = request_event.subject()?;
        log.iter().find_map(|envelope| {
            let event = Self::decode_permission_event(envelope)?;
            (event.kind == "approval_resolved"
                && event.subject().as_ref() == Some(&expected_subject))
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
        (event.kind == "permission_request" && event.subject().is_some()).then_some(event)
    }

    fn completion_key(&self, event: &Self::Event) -> CompletionKey {
        match event
            .subject()
            .expect("approval permission_request must carry canonical subject identity")
        {
            ApprovalSubject::Prompt {
                session_id,
                request_id,
            } => CompletionKey::prompt(session_id, request_id),
            ApprovalSubject::ToolCall {
                session_id,
                tool_call_id,
            } => CompletionKey::tool(session_id, tool_call_id),
        }
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

#[derive(Clone)]
struct ApprovalResolution {
    allow: bool,
    resolved_by: String,
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
            resolved_tool_calls: Arc::new(Mutex::new(HashMap::new())),
            pending_tool_calls: Arc::new(Mutex::new(HashMap::new())),
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
        let mut pending_tool_calls = HashMap::new();
        let mut resolved_tool_calls = HashMap::new();
        for envelope in &log {
            let Some(event) = ApprovalGateSubscriber::decode_permission_event(envelope) else {
                continue;
            };
            if event.session_id != *session_id {
                continue;
            }

            match event.kind.as_str() {
                "permission_request" => {
                    if let Some(request_id) = event.request_id {
                        let Some(reason) = event.reason else {
                            continue;
                        };
                        approved = false;
                        pending = Some(PendingApproval { request_id, reason });
                        continue;
                    }

                    let Some(tool_call_id) = event.tool_call_id else {
                        continue;
                    };
                    if resolved_tool_calls.contains_key(&(session_id.clone(), tool_call_id.clone()))
                    {
                        continue;
                    }
                    let Some(reason) = event.reason else {
                        continue;
                    };
                    pending_tool_calls.insert((session_id.clone(), tool_call_id), reason);
                }
                "approval_resolved" => {
                    if event.request_id.is_some() {
                        pending = None;
                        approved = event.allow.unwrap_or(false);
                        continue;
                    }

                    let Some(tool_call_id) = event.tool_call_id else {
                        continue;
                    };
                    let key = (session_id.clone(), tool_call_id);
                    pending_tool_calls.remove(&key);
                    resolved_tool_calls
                        .entry(key)
                        .or_insert(ApprovalResolution {
                            allow: event.allow.unwrap_or(false),
                            resolved_by: event
                                .resolved_by
                                .unwrap_or_else(|| "external_approval".to_string()),
                        });
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

        {
            let mut resolved = self
                .resolved_tool_calls
                .lock()
                .expect("approval state poisoned");
            resolved.retain(|(candidate_session_id, _), _| candidate_session_id != session_id);
            resolved.extend(resolved_tool_calls);
        }
        {
            let mut pending = self
                .pending_tool_calls
                .lock()
                .expect("approval state poisoned");
            pending.retain(|(candidate_session_id, _), _| candidate_session_id != session_id);
            pending.extend(pending_tool_calls);
        }

        Ok(())
    }

    async fn emit_permission_request(&self, event: PermissionEvent) -> Result<(), sacp::Error> {
        let Some(producer) = self.state_producer.as_ref() else {
            return Err(sacp::util::internal_error(
                "approval gate has no state producer; cannot emit permission_request",
            ));
        };

        producer.append_json(&self.approval_subscriber.permission_request_envelope(&event));
        producer
            .flush()
            .await
            .map_err(|error| sacp::util::internal_error(format!("approval flush: {error}")))?;
        Ok(())
    }

    /// Block until an `approval_resolved` event with a matching
    /// `request_id` appears on the state stream. Returns the
    /// resolved approval outcome, or an error if the gate timeout
    /// elapses or the stream terminates.
    async fn wait_for_approval(
        &self,
        request_event: PermissionEvent,
    ) -> Result<ApprovalResolution, sacp::Error> {
        let state_stream_url = self
            .state_stream_url
            .as_deref()
            .ok_or_else(|| sacp::util::internal_error("approval gate has no state stream URL"))?;

        let request_envelope = self
            .approval_subscriber
            .permission_request_envelope(&request_event);
        let log = self
            .subscriber_driver
            .wait_for_passive_completion(
                self.approval_subscriber.name(),
                &request_envelope,
                state_stream_url,
            )
            .await
            .map_err(|error| map_wait_error(request_event.session_id(), error))?;

        let resolution = self
            .approval_subscriber
            .completion_event_for(&request_event, &log)
            .ok_or_else(|| {
                sacp::util::internal_error(
                    "approval stream closed before the permission was resolved",
                )
            })?;

        Ok(ApprovalResolution {
            allow: resolution.allow.unwrap_or(false),
            resolved_by: resolution
                .resolved_by
                .unwrap_or_else(|| "external_approval".to_string()),
        })
    }

    fn is_session_approved(&self, session_id: &SessionId) -> bool {
        self.approved_sessions
            .lock()
            .expect("approval state poisoned")
            .contains(session_id)
    }

    fn resolved_tool_call(
        &self,
        session_id: &SessionId,
        tool_call_id: &ToolCallId,
    ) -> Option<ApprovalResolution> {
        self.resolved_tool_calls
            .lock()
            .expect("approval state poisoned")
            .get(&(session_id.clone(), tool_call_id.clone()))
            .cloned()
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

impl PermissionEvent {
    fn prompt_request(session_id: SessionId, request_id: RequestId, reason: String) -> Self {
        Self {
            kind: "permission_request".to_string(),
            session_id,
            request_id: Some(request_id),
            tool_call_id: None,
            allow: None,
            resolved_by: None,
            reason: Some(reason),
            created_at_ms: now_ms(),
            meta: None,
        }
    }

    fn tool_request(session_id: SessionId, tool_call_id: ToolCallId, reason: String) -> Self {
        Self {
            kind: "permission_request".to_string(),
            session_id,
            request_id: None,
            tool_call_id: Some(tool_call_id),
            allow: None,
            resolved_by: None,
            reason: Some(reason),
            created_at_ms: now_ms(),
            meta: None,
        }
    }

    fn prompt_resolution(
        session_id: SessionId,
        request_id: RequestId,
        allow: bool,
        resolved_by: String,
        meta: Option<TraceContext>,
    ) -> Self {
        Self {
            kind: "approval_resolved".to_string(),
            session_id,
            request_id: Some(request_id),
            tool_call_id: None,
            allow: Some(allow),
            resolved_by: Some(resolved_by),
            reason: None,
            created_at_ms: now_ms(),
            meta,
        }
    }

    fn tool_resolution(
        session_id: SessionId,
        tool_call_id: ToolCallId,
        allow: bool,
        resolved_by: String,
        meta: Option<TraceContext>,
    ) -> Self {
        Self {
            kind: "approval_resolved".to_string(),
            session_id,
            request_id: None,
            tool_call_id: Some(tool_call_id),
            allow: Some(allow),
            resolved_by: Some(resolved_by),
            reason: None,
            created_at_ms: now_ms(),
            meta,
        }
    }

    fn subject(&self) -> Option<ApprovalSubject> {
        match (&self.request_id, &self.tool_call_id) {
            (Some(request_id), None) => Some(ApprovalSubject::Prompt {
                session_id: self.session_id.clone(),
                request_id: request_id.clone(),
            }),
            (None, Some(tool_call_id)) => Some(ApprovalSubject::ToolCall {
                session_id: self.session_id.clone(),
                tool_call_id: tool_call_id.clone(),
            }),
            _ => None,
        }
    }

    fn session_id(&self) -> &SessionId {
        &self.session_id
    }
}

fn permission_request_envelope_from_event(event: PermissionEvent) -> AnyhowResult<StreamEnvelope> {
    let Some(subject) = event.subject() else {
        anyhow::bail!("permission request event must carry request_id or tool_call_id");
    };

    Ok(StreamEnvelope {
        entity_type: "permission".to_string(),
        key: subject.stream_key(),
        headers: insert_headers(),
        source_offset: None,
        value: Some(serde_json::to_value(event)?),
    })
}

pub fn permission_request_envelope(
    session_id: SessionId,
    request_id: RequestId,
    reason: String,
) -> AnyhowResult<StreamEnvelope> {
    permission_request_envelope_from_event(PermissionEvent::prompt_request(
        session_id, request_id, reason,
    ))
}

pub fn tool_permission_request_envelope(
    session_id: SessionId,
    tool_call_id: ToolCallId,
    reason: String,
) -> AnyhowResult<StreamEnvelope> {
    permission_request_envelope_from_event(PermissionEvent::tool_request(
        session_id,
        tool_call_id,
        reason,
    ))
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
    approval_resolution_envelope_from_event(PermissionEvent::prompt_resolution(
        session_id,
        request_id,
        allow,
        resolved_by,
        meta,
    ))
}

pub fn tool_approval_resolution_envelope(
    session_id: SessionId,
    tool_call_id: ToolCallId,
    allow: bool,
    resolved_by: String,
) -> AnyhowResult<StreamEnvelope> {
    tool_approval_resolution_envelope_with_trace(session_id, tool_call_id, allow, resolved_by, None)
}

pub fn tool_approval_resolution_envelope_with_trace(
    session_id: SessionId,
    tool_call_id: ToolCallId,
    allow: bool,
    resolved_by: String,
    meta: Option<TraceContext>,
) -> AnyhowResult<StreamEnvelope> {
    approval_resolution_envelope_from_event(PermissionEvent::tool_resolution(
        session_id,
        tool_call_id,
        allow,
        resolved_by,
        meta,
    ))
}

fn approval_resolution_envelope_from_event(event: PermissionEvent) -> AnyhowResult<StreamEnvelope> {
    let Some(subject) = event.subject() else {
        anyhow::bail!("approval resolution event must carry request_id or tool_call_id");
    };

    Ok(StreamEnvelope {
        entity_type: "permission".to_string(),
        key: subject.resolved_stream_key(),
        headers: insert_headers(),
        source_offset: None,
        value: Some(serde_json::to_value(event)?),
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
                            policy.match_rule.matches_prompt(&prompt_text).then(|| {
                                (policy.action, policy.reason.clone(), policy.policy_id())
                            })
                        });

                        let Some((action, reason, policy_id)) = policy_match else {
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
                                let request_event = PermissionEvent::prompt_request(
                                    session_id.clone(),
                                    request_id.clone(),
                                    reason.clone(),
                                );
                                ensure_prompt_request_span(&session_id, &request_id, Some("session/prompt"));
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
                                    start_approval_requested_span(
                                        &session_id,
                                        &request_id,
                                        &policy_id,
                                        &reason,
                                    );
                                    if let Err(error) = this.emit_permission_request(request_event.clone()).await {
                                        clear_approval_requested_span(&session_id, &request_id);
                                        this.pending_sessions
                                            .lock()
                                            .expect("approval state poisoned")
                                            .remove(&session_id);
                                        return responder.respond_with_error(error);
                                    }
                                }
                                let resolution = match this.wait_for_approval(request_event).await {
                                    Ok(resolution) => resolution,
                                    Err(error) => {
                                        clear_approval_requested_span(&session_id, &request_id);
                                        this.pending_sessions
                                            .lock()
                                            .expect("approval state poisoned")
                                            .remove(&session_id);
                                        return responder.respond_with_error(error);
                                    }
                                };
                                emit_approval_resolved_span(
                                    &session_id,
                                    &request_id,
                                    resolution.allow,
                                    &resolution.resolved_by,
                                );
                                this.pending_sessions
                                    .lock()
                                    .expect("approval state poisoned")
                                    .remove(&session_id);
                                if !resolution.allow {
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
                Agent,
                {
                    let this = this.clone();
                    async move |request: RequestPermissionRequest, responder, cx| {
                        let session_id = request.session_id.clone();
                        let tool_call_id = request.tool_call.tool_call_id.clone();
                        let tool_title = request.tool_call.fields.title.clone().unwrap_or_default();

                        let Some((action, reason)) = this
                            .config
                            .policy_for_tool(&tool_title)
                            .map(|policy| (policy.action, policy.reason.clone()))
                        else {
                            return cx
                                .send_request_to(Client, request)
                                .forward_response_to(responder);
                        };

                        let tool_key = (session_id.clone(), tool_call_id.clone());
                        if let Some(resolution) =
                            this.resolved_tool_call(&session_id, &tool_call_id)
                        {
                            return responder
                                .respond(tool_permission_response(&request.options, resolution.allow));
                        }

                        match action {
                            ApprovalAction::Deny => responder.respond(RequestPermissionResponse::new(
                                tool_permission_outcome(&request.options, false),
                            )),
                            ApprovalAction::RequireApproval => {
                                let request_event = PermissionEvent::tool_request(
                                    session_id.clone(),
                                    tool_call_id.clone(),
                                    reason.clone(),
                                );

                                let should_emit = {
                                    let mut pending = this
                                        .pending_tool_calls
                                        .lock()
                                        .expect("approval state poisoned");
                                    let should_emit = !pending.contains_key(&tool_key);
                                    pending.insert(tool_key.clone(), reason.clone());
                                    should_emit
                                };

                                if should_emit {
                                    if let Err(error) =
                                        this.emit_permission_request(request_event.clone()).await
                                    {
                                        this.pending_tool_calls
                                            .lock()
                                            .expect("approval state poisoned")
                                            .remove(&tool_key);
                                        return responder.respond_with_error(error);
                                    }
                                }

                                let resolution = match this.wait_for_approval(request_event).await {
                                    Ok(resolution) => resolution,
                                    Err(error) => {
                                        this.pending_tool_calls
                                            .lock()
                                            .expect("approval state poisoned")
                                            .remove(&tool_key);
                                        return responder.respond_with_error(error);
                                    }
                                };

                                this.pending_tool_calls
                                    .lock()
                                    .expect("approval state poisoned")
                                    .remove(&tool_key);
                                this.resolved_tool_calls
                                    .lock()
                                    .expect("approval state poisoned")
                                    .insert(tool_key, resolution.clone());

                                responder.respond(tool_permission_response(
                                    &request.options,
                                    resolution.allow,
                                ))
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

fn tool_permission_response(
    options: &[PermissionOption],
    allow: bool,
) -> RequestPermissionResponse {
    RequestPermissionResponse::new(tool_permission_outcome(options, allow))
}

fn tool_permission_outcome(options: &[PermissionOption], allow: bool) -> RequestPermissionOutcome {
    let selected = options.iter().find(|option| match option.kind {
        PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways => allow,
        PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways => !allow,
        _ => false,
    });

    match selected {
        Some(option) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
            option.option_id.clone(),
        )),
        None => RequestPermissionOutcome::Cancelled,
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

    async fn append_tool_approval_resolved_event(
        producer: &Producer,
        session_id: &str,
        tool_call_id: &str,
        allow: bool,
    ) -> Result<()> {
        producer.append_json(&tool_approval_resolution_envelope(
            SessionId::from(session_id.to_string()),
            ToolCallId::from(tool_call_id.to_string()),
            allow,
            "approval-test".to_string(),
        )?);
        producer
            .flush()
            .await
            .context("flush tool-scoped approval_resolved test event")?;
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
                gate.wait_for_approval(PermissionEvent::prompt_request(
                    SessionId::from("session-a"),
                    RequestId::from("request-a".to_string()),
                    "approval".to_string(),
                ))
                .await
            }
        });
        let mut waiter_b = tokio::spawn({
            let gate = gate.clone();
            async move {
                gate.wait_for_approval(PermissionEvent::prompt_request(
                    SessionId::from("session-b"),
                    RequestId::from("request-b".to_string()),
                    "approval".to_string(),
                ))
                .await
            }
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        append_approval_resolved_event(&producer, "session-a", "request-a", true).await?;

        let allow_a = tokio::time::timeout(Duration::from_secs(2), waiter_a)
            .await
            .context("session A waiter did not resolve after matching approval")?
            .context("session A waiter panicked")??;
        assert!(
            allow_a.allow,
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
            allow_b.allow,
            "INVARIANT (ApprovalGate): session B must release once its own approval_resolved arrives"
        );

        server.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn tool_call_waiters_are_isolated_by_session_and_tool_call_id() -> Result<()> {
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
                gate.wait_for_approval(PermissionEvent::tool_request(
                    SessionId::from("session-a"),
                    ToolCallId::from("tool-a".to_string()),
                    "tool approval".to_string(),
                ))
                .await
            }
        });
        let mut waiter_b = tokio::spawn({
            let gate = gate.clone();
            async move {
                gate.wait_for_approval(PermissionEvent::tool_request(
                    SessionId::from("session-a"),
                    ToolCallId::from("tool-b".to_string()),
                    "tool approval".to_string(),
                ))
                .await
            }
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        append_tool_approval_resolved_event(&producer, "session-a", "tool-a", true).await?;

        let allow_a = tokio::time::timeout(Duration::from_secs(2), waiter_a)
            .await
            .context("tool waiter A did not resolve after matching approval")?
            .context("tool waiter A panicked")??;
        assert!(
            allow_a.allow,
            "INVARIANT (ApprovalGate): matching tool_call_id approval_resolved must release the corresponding waiter"
        );

        assert!(
            tokio::time::timeout(Duration::from_millis(250), &mut waiter_b)
                .await
                .is_err(),
            "INVARIANT (ApprovalGate): resolving tool A must not release tool B's waiter"
        );

        append_tool_approval_resolved_event(&producer, "session-a", "tool-b", true).await?;
        let allow_b = tokio::time::timeout(Duration::from_secs(2), waiter_b)
            .await
            .context("tool waiter B did not resolve after its own approval")?
            .context("tool waiter B panicked")??;
        assert!(
            allow_b.allow,
            "INVARIANT (ApprovalGate): tool waiter B must release once its own tool_call_id approval_resolved arrives"
        );

        server.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn rebuild_from_log_keeps_resolved_tool_call_completed_after_duplicate_request()
    -> Result<()> {
        let server = TestStreamServer::spawn().await?;
        let stream_url = server.stream_url(&format!("approval-gate-{}", uuid::Uuid::new_v4()));
        ensure_json_stream_exists(&stream_url).await?;
        let producer = test_permission_producer(&stream_url);
        seed_permission_stream(&producer).await?;

        producer.append_json(&tool_permission_request_envelope(
            SessionId::from("session-a"),
            ToolCallId::from("tool-1".to_string()),
            "tool approval".to_string(),
        )?);
        producer.append_json(&tool_approval_resolution_envelope(
            SessionId::from("session-a"),
            ToolCallId::from("tool-1".to_string()),
            true,
            "approval-test".to_string(),
        )?);
        producer.append_json(&tool_permission_request_envelope(
            SessionId::from("session-a"),
            ToolCallId::from("tool-1".to_string()),
            "duplicate replay".to_string(),
        )?);
        producer
            .flush()
            .await
            .context("flush tool replay sequence")?;

        let gate = ApprovalGateComponent::with_stream_and_timeout(
            ApprovalConfig::default(),
            Some(stream_url),
            None,
            Some(Duration::from_secs(5)),
        );

        gate.rebuild_from_log(&SessionId::from("session-a")).await?;

        assert!(
            gate.resolved_tool_call(
                &SessionId::from("session-a"),
                &ToolCallId::from("tool-1".to_string())
            )
            .is_some(),
            "INVARIANT (ApprovalGate): replay should preserve completed tool_call_id approvals after session/load rebuild",
        );
        assert!(
            !gate
                .pending_tool_calls
                .lock()
                .expect("approval state poisoned")
                .contains_key(&(
                    SessionId::from("session-a"),
                    ToolCallId::from("tool-1".to_string())
                )),
            "INVARIANT (ApprovalGate): duplicate replayed permission_request must not re-open a completed tool_call_id approval",
        );

        server.shutdown().await;
        Ok(())
    }
}
