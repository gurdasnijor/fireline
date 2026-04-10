use std::collections::{HashMap, HashSet};

use sacp_conductor::trace::{NotificationEvent, RequestEvent, ResponseEvent, TraceEvent};
use serde::Serialize;
use serde_json::Value;

use crate::session::{SessionRecord, SessionStatus};

pub type StateChange = Value;

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

pub struct StateProjector {
    runtime_key: String,
    runtime_id: String,
    node_id: String,
    logical_connection_id: String,
    connection: ConnectionRow,
    correlation: TraceCorrelationState,
    inherited_lineage: InheritedLineage,
    supports_load_session: bool,
}

impl StateProjector {
    pub fn new(
        runtime_key: impl Into<String>,
        runtime_id: impl Into<String>,
        node_id: impl Into<String>,
        logical_connection_id: impl Into<String>,
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

        Self {
            runtime_key,
            runtime_id,
            node_id,
            logical_connection_id,
            connection,
            correlation: TraceCorrelationState::default(),
            inherited_lineage: InheritedLineage::default(),
            supports_load_session: false,
        }
    }

    pub fn initial_events(&self) -> Vec<StateChange> {
        vec![state_change(
            "connection",
            &self.logical_connection_id,
            "insert",
            Some(&self.connection),
        )]
    }

    pub fn project_trace_event(&mut self, event: &TraceEvent) -> Vec<StateChange> {
        match event {
            TraceEvent::Request(req) => self.handle_request(req),
            TraceEvent::Response(resp) => self.handle_response(resp),
            TraceEvent::Notification(notif) => self.handle_notification(notif),
            _ => Vec::new(),
        }
    }

    fn handle_request(&mut self, req: &RequestEvent) -> Vec<StateChange> {
        match req.method.as_str() {
            "initialize" | "_proxy/initialize" => {
                if !is_canonical_client_request(req) {
                    return Vec::new();
                }
                self.correlation
                    .pending_initialize
                    .insert(req.id.to_string());
                self.inherited_lineage = parse_fireline_lineage(&req.params);
                Vec::new()
            }
            "session/new" => {
                if !is_canonical_client_request(req) {
                    return Vec::new();
                }
                let request_id = req.id.to_string();
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
                vec![state_change(
                    "pending_request",
                    &request_id,
                    "insert",
                    Some(&pending),
                )]
            }
            "session/prompt" => {
                if !is_canonical_client_request(req) {
                    return Vec::new();
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
                    trace_id: Some(trace_id),
                    parent_prompt_turn_id,
                    text,
                    state: PromptTurnState::Active,
                    position: None,
                    stop_reason: None,
                    started_at: now_ms(),
                    completed_at: None,
                };
                self.correlation
                    .prompt_turns
                    .insert(prompt_turn_id.clone(), turn.clone());

                let pending = PendingRequestRow {
                    request_id: request_id.clone(),
                    logical_connection_id: self.logical_connection_id.clone(),
                    session_id: Some(session_id),
                    prompt_turn_id: Some(prompt_turn_id.clone()),
                    method: req.method.clone(),
                    direction: PendingRequestDirection::ClientToAgent,
                    state: PendingRequestState::Pending,
                    created_at: now_ms(),
                    resolved_at: None,
                };
                self.correlation
                    .pending_requests
                    .insert(request_id.clone(), pending.clone());

                vec![
                    state_change("prompt_turn", &prompt_turn_id, "insert", Some(&turn)),
                    state_change("pending_request", &request_id, "insert", Some(&pending)),
                ]
            }
            _ => Vec::new(),
        }
    }

    fn handle_response(&mut self, resp: &ResponseEvent) -> Vec<StateChange> {
        let request_id = resp.id.to_string();
        let mut changes = Vec::new();

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
            changes.push(state_change(
                "pending_request",
                &request_id,
                "update",
                Some(&pending),
            ));

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
                    changes.push(state_change(
                        "session",
                        session_id,
                        "insert",
                        Some(&session),
                    ));

                    self.connection.state = ConnectionState::Attached;
                    self.connection.latest_session_id = Some(session_id.to_string());
                    self.connection.last_error = None;
                }

                self.connection.updated_at = now_ms();
                changes.push(state_change(
                    "connection",
                    &self.logical_connection_id,
                    "update",
                    Some(&self.connection),
                ));
                return changes;
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
                return changes;
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

            changes.push(state_change(
                "prompt_turn",
                &prompt_turn_id,
                "update",
                Some(&turn),
            ));
        }

        changes
    }

    fn handle_notification(&mut self, notif: &NotificationEvent) -> Vec<StateChange> {
        if notif.method != "session/update" || !is_canonical_session_update_notification(notif) {
            return Vec::new();
        }

        let session_id = notif
            .session
            .as_deref()
            .or_else(|| notif.params.get("sessionId").and_then(Value::as_str))
            .or_else(|| notif.params.get("session_id").and_then(Value::as_str));
        let Some(session_id) = session_id else {
            return Vec::new();
        };
        let Some(prompt_turn_id) = self
            .correlation
            .session_active_turn
            .get(session_id)
            .cloned()
        else {
            return Vec::new();
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
            _ => return Vec::new(),
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

        vec![state_change(
            "chunk",
            &chunk.chunk_id,
            "insert",
            Some(&chunk),
        )]
    }

    fn next_prompt_turn_id(&mut self) -> String {
        self.correlation.turn_counter += 1;
        format!(
            "{}:{}:{}",
            self.runtime_id, self.logical_connection_id, self.correlation.turn_counter
        )
    }
}

pub fn runtime_instance_started(
    runtime_id: &str,
    runtime_name: &str,
    created_at: i64,
) -> StateChange {
    let row = RuntimeInstanceRow {
        instance_id: runtime_id.to_string(),
        runtime_name: runtime_name.to_string(),
        status: RuntimeInstanceState::Running,
        created_at,
        updated_at: created_at,
    };
    state_change("runtime_instance", runtime_id, "insert", Some(&row))
}

pub fn runtime_instance_stopped(
    runtime_id: &str,
    runtime_name: &str,
    created_at: i64,
) -> StateChange {
    let row = RuntimeInstanceRow {
        instance_id: runtime_id.to_string(),
        runtime_name: runtime_name.to_string(),
        status: RuntimeInstanceState::Stopped,
        created_at,
        updated_at: now_ms(),
    };
    state_change("runtime_instance", runtime_id, "update", Some(&row))
}

fn state_change<T: Serialize>(
    entity_type: &'static str,
    key: &str,
    operation: &'static str,
    value: Option<&T>,
) -> StateChange {
    serde_json::to_value(StateEnvelope {
        entity_type,
        key: key.to_string(),
        headers: StateHeaders { operation },
        value,
    })
    .expect("serialize state envelope")
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
