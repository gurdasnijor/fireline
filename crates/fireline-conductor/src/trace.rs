//! Durable stream state writer.
//!
//! [`DurableStreamTracer`] implements [`sacp_conductor::trace::WriteEvent`]
//! and is the destination for `ConductorImpl::trace_to(...)`.
//!
//! It observes ACP trace events, correlates them into normalized
//! `STATE-PROTOCOL` entity changes, and appends those changes to the
//! Fireline durable state stream.
//!
//! Important nuance:
//!
//! - active components stamp protocol extensions into ACP `_meta`
//! - this writer may observe `_meta`, but it does not invent or mutate it
//! - the durable stream carries normalized state rows, not raw `TraceEvent`
//!   envelopes

use std::collections::{HashMap, HashSet};
use std::io;

use durable_streams::Producer;
use sacp_conductor::trace::{
    NotificationEvent, RequestEvent, ResponseEvent, TraceEvent, WriteEvent,
};
use serde::Serialize;
use serde_json::Value;

use crate::lineage::LineageTracker;
use crate::session::{SessionRecord, SessionStatus};

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
#[serde(rename_all = "snake_case")]
enum ConnectionState {
    Created,
    Attached,
    Broken,
    Closed,
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
#[serde(rename_all = "snake_case")]
enum PromptTurnState {
    Queued,
    Active,
    Completed,
    CancelRequested,
    Cancelled,
    Broken,
    TimedOut,
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
#[serde(rename_all = "snake_case")]
enum PendingRequestState {
    Pending,
    Resolved,
    Orphaned,
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
#[serde(rename_all = "snake_case")]
enum RuntimeInstanceState {
    Running,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum ChunkType {
    Text,
    ToolCall,
    Thinking,
    ToolResult,
    Error,
    Stop,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionRow {
    logical_connection_id: String,
    state: ConnectionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    queue_paused: Option<bool>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptTurnRow {
    prompt_turn_id: String,
    logical_connection_id: String,
    session_id: String,
    request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_prompt_turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    state: PromptTurnState,
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_reason: Option<String>,
    started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingRequestRow {
    request_id: String,
    logical_connection_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_turn_id: Option<String>,
    method: String,
    direction: PendingRequestDirection,
    state: PendingRequestState,
    created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
#[serde(rename_all = "snake_case")]
enum PendingRequestDirection {
    ClientToAgent,
    AgentToClient,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeInstanceRow {
    instance_id: String,
    runtime_name: String,
    status: RuntimeInstanceState,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChunkRow {
    chunk_id: String,
    prompt_turn_id: String,
    logical_connection_id: String,
    #[serde(rename = "type")]
    chunk_type: ChunkType,
    content: String,
    seq: i64,
    created_at: i64,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<T>,
}

#[derive(Debug, Default)]
struct TraceCorrelationState {
    pending_initialize: HashSet<String>,
    prompt_request_to_turn: HashMap<String, String>,
    prompt_turns: HashMap<String, PromptTurnRow>,
    pending_requests: HashMap<String, PendingRequestRow>,
    session_active_turn: HashMap<String, String>,
    chunk_seq: HashMap<String, i64>,
    turn_counter: u64,
}

#[derive(Debug, Clone, Default)]
struct InheritedLineage {
    trace_id: Option<String>,
    parent_prompt_turn_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TraceEndpoint {
    Client,
    Agent,
    Proxy(usize),
    Unknown,
}

pub struct DurableStreamTracer {
    runtime_key: String,
    producer: Producer,
    runtime_id: String,
    node_id: String,
    logical_connection_id: String,
    connection: ConnectionRow,
    correlation: TraceCorrelationState,
    inherited_lineage: InheritedLineage,
    lineage_tracker: LineageTracker,
    supports_load_session: bool,
}

impl DurableStreamTracer {
    pub fn new(
        producer: Producer,
        runtime_id: impl Into<String>,
        logical_connection_id: impl Into<String>,
    ) -> Self {
        let runtime_id = runtime_id.into();
        Self::new_with_runtime_context(
            producer,
            runtime_id.clone(),
            runtime_id,
            "node:unknown",
            logical_connection_id,
            LineageTracker::default(),
        )
    }

    pub fn new_with_tracker(
        producer: Producer,
        runtime_id: impl Into<String>,
        logical_connection_id: impl Into<String>,
        lineage_tracker: LineageTracker,
    ) -> Self {
        let runtime_id = runtime_id.into();
        Self::new_with_runtime_context(
            producer,
            runtime_id.clone(),
            runtime_id,
            "node:unknown",
            logical_connection_id,
            lineage_tracker,
        )
    }

    pub fn new_with_runtime_context(
        producer: Producer,
        runtime_key: impl Into<String>,
        runtime_id: impl Into<String>,
        node_id: impl Into<String>,
        logical_connection_id: impl Into<String>,
        lineage_tracker: LineageTracker,
    ) -> Self {
        let runtime_key = runtime_key.into();
        let runtime_id = runtime_id.into();
        let node_id = node_id.into();
        let logical_connection_id = logical_connection_id.into();
        let now = now_ms();
        let connection = ConnectionRow {
            logical_connection_id: logical_connection_id.clone(),
            state: ConnectionState::Created,
            latest_session_id: None,
            last_error: None,
            queue_paused: None,
            created_at: now,
            updated_at: now,
        };

        append_state(
            &producer,
            "connection",
            &logical_connection_id,
            "insert",
            Some(&connection),
        );

        Self {
            runtime_key,
            producer,
            runtime_id,
            node_id,
            logical_connection_id,
            connection,
            correlation: TraceCorrelationState::default(),
            inherited_lineage: InheritedLineage::default(),
            lineage_tracker,
            supports_load_session: false,
        }
    }
}

impl WriteEvent for DurableStreamTracer {
    fn write_event(&mut self, event: &TraceEvent) -> io::Result<()> {
        match event {
            TraceEvent::Request(req) => self.handle_request(req),
            TraceEvent::Response(resp) => self.handle_response(resp),
            TraceEvent::Notification(notif) => self.handle_notification(notif),
            _ => {}
        }

        Ok(())
    }
}

impl DurableStreamTracer {
    fn handle_request(&mut self, req: &RequestEvent) {
        match req.method.as_str() {
            "initialize" | "_proxy/initialize" => {
                if !is_canonical_client_request(req) {
                    return;
                }
                self.correlation
                    .pending_initialize
                    .insert(req.id.to_string());
                self.inherited_lineage = parse_fireline_lineage(&req.params);
            }
            "session/new" => {
                let request_id = req.id.to_string();
                if !is_canonical_client_request(req) {
                    return;
                }

                let pending = PendingRequestRow {
                    request_id: request_id.clone(),
                    logical_connection_id: self.logical_connection_id.clone(),
                    session_id: None,
                    prompt_turn_id: None,
                    method: req.method.clone(),
                    direction: PendingRequestDirection::ClientToAgent,
                    state: PendingRequestState::Pending,
                    created_at: now_ms(),
                    resolved_at: None,
                };
                self.correlation
                    .pending_requests
                    .insert(request_id.clone(), pending.clone());
                append_state(
                    &self.producer,
                    "pending_request",
                    &request_id,
                    "insert",
                    Some(&pending),
                );
            }
            "session/prompt" => {
                if !is_canonical_client_request(req) {
                    return;
                }
                let session_id = req
                    .params
                    .get("sessionId")
                    .or_else(|| req.params.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                let request_id = req.id.to_string();
                let prompt_turn_id = self.next_prompt_turn_id();
                let trace_id = self
                    .inherited_lineage
                    .trace_id
                    .clone()
                    .unwrap_or_else(|| prompt_turn_id.clone());
                let parent_prompt_turn_id = self.inherited_lineage.parent_prompt_turn_id.clone();
                let text = req
                    .params
                    .get("prompt")
                    .and_then(Value::as_array)
                    .and_then(|blocks| first_text_block(blocks));

                self.correlation
                    .prompt_request_to_turn
                    .insert(request_id.clone(), prompt_turn_id.clone());
                self.correlation
                    .session_active_turn
                    .insert(session_id.clone(), prompt_turn_id.clone());

                let turn = PromptTurnRow {
                    prompt_turn_id: prompt_turn_id.clone(),
                    logical_connection_id: self.logical_connection_id.clone(),
                    session_id: session_id.clone(),
                    request_id: request_id.clone(),
                    trace_id: Some(trace_id.clone()),
                    parent_prompt_turn_id,
                    text,
                    state: PromptTurnState::Active,
                    position: None,
                    stop_reason: None,
                    started_at: now_ms(),
                    completed_at: None,
                };
                append_state(
                    &self.producer,
                    "prompt_turn",
                    &prompt_turn_id,
                    "insert",
                    Some(&turn),
                );
                self.correlation
                    .prompt_turns
                    .insert(prompt_turn_id.clone(), turn);
                self.lineage_tracker
                    .note_active_turn(&session_id, &trace_id, &prompt_turn_id);

                let pending = PendingRequestRow {
                    request_id: request_id.clone(),
                    logical_connection_id: self.logical_connection_id.clone(),
                    session_id: Some(session_id),
                    prompt_turn_id: Some(prompt_turn_id),
                    method: req.method.clone(),
                    direction: PendingRequestDirection::ClientToAgent,
                    state: PendingRequestState::Pending,
                    created_at: now_ms(),
                    resolved_at: None,
                };
                self.correlation
                    .pending_requests
                    .insert(request_id.clone(), pending.clone());
                append_state(
                    &self.producer,
                    "pending_request",
                    &request_id,
                    "insert",
                    Some(&pending),
                );
            }
            _ => {}
        }
    }

    fn handle_response(&mut self, resp: &ResponseEvent) {
        let request_id = resp.id.to_string();

        if self.correlation.pending_initialize.remove(&request_id) {
            self.supports_load_session = resp
                .payload
                .get("agentCapabilities")
                .or_else(|| resp.payload.get("agent_capabilities"))
                .and_then(|caps| caps.get("loadSession").or_else(|| caps.get("load_session")))
                .and_then(Value::as_bool)
                .unwrap_or(false);
        }

        if let Some(mut pending) = self.correlation.pending_requests.remove(&request_id) {
            let was_session_new = pending.method == "session/new";
            pending.state = PendingRequestState::Resolved;
            pending.resolved_at = Some(now_ms());
            append_state(
                &self.producer,
                "pending_request",
                &request_id,
                "update",
                Some(&pending),
            );

            if was_session_new {
                if resp.is_error {
                    self.connection.state = ConnectionState::Broken;
                    self.connection.last_error = Some(resp.payload.to_string());
                } else if let Some(session_id) = resp
                    .payload
                    .get("sessionId")
                    .or_else(|| resp.payload.get("session_id"))
                    .and_then(Value::as_str)
                {
                    let now = now_ms();
                    let session = SessionRecord {
                        session_id: session_id.to_string(),
                        runtime_key: self.runtime_key.clone(),
                        runtime_id: self.runtime_id.clone(),
                        node_id: self.node_id.clone(),
                        logical_connection_id: self.logical_connection_id.clone(),
                        state: SessionStatus::Active,
                        supports_load_session: self.supports_load_session,
                        trace_id: self.inherited_lineage.trace_id.clone(),
                        parent_prompt_turn_id: self.inherited_lineage.parent_prompt_turn_id.clone(),
                        created_at: now,
                        updated_at: now,
                        last_seen_at: now,
                    };
                    append_state(
                        &self.producer,
                        "session",
                        session_id,
                        "insert",
                        Some(&session),
                    );

                    self.connection.state = ConnectionState::Attached;
                    self.connection.latest_session_id = Some(session_id.to_string());
                    self.connection.last_error = None;
                }

                self.connection.updated_at = now_ms();
                append_state(
                    &self.producer,
                    "connection",
                    &self.logical_connection_id,
                    "update",
                    Some(&self.connection),
                );
                return;
            }
        }

        if let Some(prompt_turn_id) = self.correlation.prompt_request_to_turn.remove(&request_id) {
            let stop_reason = if resp.is_error {
                Some("error".to_string())
            } else {
                resp.payload
                    .get("stopReason")
                    .or_else(|| resp.payload.get("stop_reason"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            };

            let Some(mut turn) = self.correlation.prompt_turns.get(&prompt_turn_id).cloned() else {
                return;
            };
            turn.state = if resp.is_error {
                PromptTurnState::Broken
            } else {
                PromptTurnState::Completed
            };
            turn.stop_reason = stop_reason;
            turn.completed_at = Some(now_ms());
            self.correlation
                .prompt_turns
                .insert(prompt_turn_id.clone(), turn.clone());

            self.correlation
                .session_active_turn
                .retain(|_, value| value != &prompt_turn_id);
            self.correlation.chunk_seq.remove(&prompt_turn_id);
            self.lineage_tracker
                .clear_active_turn(&turn.session_id, &prompt_turn_id);

            append_state(
                &self.producer,
                "prompt_turn",
                &prompt_turn_id,
                "update",
                Some(&turn),
            );
        }
    }

    fn handle_notification(&mut self, notif: &NotificationEvent) {
        if notif.method != "session/update" || !is_canonical_session_update_notification(notif) {
            return;
        }

        let session_id = notif
            .session
            .as_deref()
            .or_else(|| notif.params.get("sessionId").and_then(Value::as_str))
            .or_else(|| notif.params.get("session_id").and_then(Value::as_str));
        let Some(session_id) = session_id else {
            return;
        };
        let Some(prompt_turn_id) = self
            .correlation
            .session_active_turn
            .get(session_id)
            .cloned()
        else {
            return;
        };

        let update = notif.params.get("update");
        let update_type = update
            .and_then(|u| u.get("sessionUpdate").or_else(|| u.get("type")))
            .and_then(Value::as_str);

        let (chunk_type, content) = match update_type {
            Some("agent_message_chunk")
            | Some("agentMessageChunk")
            | Some("user_message_chunk")
            | Some("userMessageChunk") => {
                let text = update
                    .and_then(|u| u.get("content"))
                    .and_then(|c| c.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                (ChunkType::Text, text.to_string())
            }
            Some("agent_thought_chunk") | Some("agentThoughtChunk") => {
                let text = update
                    .and_then(|u| u.get("content"))
                    .and_then(|c| c.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                (ChunkType::Thinking, text.to_string())
            }
            Some("tool_call") | Some("toolCall") => (
                ChunkType::ToolCall,
                update.map(Value::to_string).unwrap_or_default(),
            ),
            Some("tool_call_update") | Some("toolCallUpdate") => (
                ChunkType::ToolResult,
                update.map(Value::to_string).unwrap_or_default(),
            ),
            Some("agent_error") | Some("agentError") => (
                ChunkType::Error,
                update.map(Value::to_string).unwrap_or_default(),
            ),
            Some("stop") => (ChunkType::Stop, String::new()),
            _ => return,
        };

        let seq = self
            .correlation
            .chunk_seq
            .entry(prompt_turn_id.clone())
            .or_insert(0);
        let current_seq = *seq;
        *seq += 1;

        let chunk = ChunkRow {
            chunk_id: uuid::Uuid::new_v4().to_string(),
            prompt_turn_id,
            logical_connection_id: self.logical_connection_id.clone(),
            chunk_type,
            content,
            seq: current_seq,
            created_at: now_ms(),
        };
        append_state(
            &self.producer,
            "chunk",
            &chunk.chunk_id,
            "insert",
            Some(&chunk),
        );
    }

    fn next_prompt_turn_id(&mut self) -> String {
        self.correlation.turn_counter += 1;
        format!(
            "{}:{}:{}",
            self.runtime_id, self.logical_connection_id, self.correlation.turn_counter
        )
    }
}

pub fn emit_runtime_instance_started(
    producer: &Producer,
    runtime_id: &str,
    runtime_name: &str,
    created_at: i64,
) {
    let row = RuntimeInstanceRow {
        instance_id: runtime_id.to_string(),
        runtime_name: runtime_name.to_string(),
        status: RuntimeInstanceState::Running,
        created_at,
        updated_at: created_at,
    };
    append_state(
        producer,
        "runtime_instance",
        runtime_id,
        "insert",
        Some(&row),
    );
}

pub fn emit_runtime_instance_stopped(
    producer: &Producer,
    runtime_id: &str,
    runtime_name: &str,
    created_at: i64,
) {
    let row = RuntimeInstanceRow {
        instance_id: runtime_id.to_string(),
        runtime_name: runtime_name.to_string(),
        status: RuntimeInstanceState::Stopped,
        created_at,
        updated_at: now_ms(),
    };
    append_state(
        producer,
        "runtime_instance",
        runtime_id,
        "update",
        Some(&row),
    );
}

fn append_state<T: Serialize>(
    producer: &Producer,
    entity_type: &'static str,
    key: &str,
    operation: &'static str,
    value: Option<&T>,
) {
    let envelope = StateEnvelope {
        entity_type,
        key: key.to_string(),
        headers: StateHeaders { operation },
        value,
    };
    producer.append_json(&envelope);
}

fn first_text_block(blocks: &[Value]) -> Option<String> {
    blocks.iter().find_map(|block| {
        if block.get("type").and_then(Value::as_str) == Some("text") {
            block
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
        } else {
            None
        }
    })
}

fn parse_fireline_lineage(params: &Value) -> InheritedLineage {
    let top_level_meta = params.get("_meta").and_then(Value::as_object);
    let client_meta = params
        .get("clientCapabilities")
        .or_else(|| params.get("client_capabilities"))
        .and_then(|caps| caps.get("_meta").or_else(|| caps.get("meta")))
        .and_then(Value::as_object);

    let trace_id = top_level_meta
        .and_then(|meta| meta.get("fireline"))
        .and_then(Value::as_object)
        .and_then(|fireline| fireline.get("traceId"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            top_level_meta
                .and_then(|meta| meta.get("fireline/trace-id"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            client_meta
                .and_then(|meta| meta.get("fireline"))
                .and_then(Value::as_object)
                .and_then(|fireline| fireline.get("traceId"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            client_meta
                .and_then(|meta| meta.get("fireline/trace-id"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    let parent_prompt_turn_id = top_level_meta
        .and_then(|meta| meta.get("fireline"))
        .and_then(Value::as_object)
        .and_then(|fireline| fireline.get("parentPromptTurnId"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            top_level_meta
                .and_then(|meta| meta.get("fireline/parent-prompt-turn-id"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            client_meta
                .and_then(|meta| meta.get("fireline"))
                .and_then(Value::as_object)
                .and_then(|fireline| fireline.get("parentPromptTurnId"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            client_meta
                .and_then(|meta| meta.get("fireline/parent-prompt-turn-id"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    InheritedLineage {
        trace_id,
        parent_prompt_turn_id,
    }
}

fn parse_trace_endpoint(raw: &str) -> TraceEndpoint {
    let value = raw.trim();

    if value.eq_ignore_ascii_case("client") {
        return TraceEndpoint::Client;
    }
    if value.eq_ignore_ascii_case("agent") {
        return TraceEndpoint::Agent;
    }

    let lower = value.to_ascii_lowercase();
    if let Some(idx) = lower
        .strip_prefix("proxy(")
        .and_then(|s| s.strip_suffix(')'))
        .and_then(|s| s.parse::<usize>().ok())
    {
        return TraceEndpoint::Proxy(idx);
    }
    if let Some(idx) = lower
        .strip_prefix("proxy:")
        .and_then(|s| s.parse::<usize>().ok())
    {
        return TraceEndpoint::Proxy(idx);
    }

    TraceEndpoint::Unknown
}

fn is_canonical_client_request(req: &RequestEvent) -> bool {
    let from = parse_trace_endpoint(&req.from);
    let to = parse_trace_endpoint(&req.to);
    from == TraceEndpoint::Client && matches!(to, TraceEndpoint::Proxy(0) | TraceEndpoint::Agent)
}

fn is_canonical_session_update_notification(notif: &NotificationEvent) -> bool {
    matches!(
        parse_trace_endpoint(&notif.to),
        TraceEndpoint::Proxy(0) | TraceEndpoint::Client
    )
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}
