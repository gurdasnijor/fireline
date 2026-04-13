use std::collections::{HashMap, HashSet};

use sacp::schema::{SessionUpdate, StopReason};
use sacp_conductor::trace::{NotificationEvent, RequestEvent, ResponseEvent, TraceEvent};
use serde::Serialize;
use serde_json::Value;
use fireline_acp_ids::{PromptRequestRef, RequestId, SessionId, ToolCallId};
use fireline_session::{SessionRecord, SessionStatus};

pub type StateChange = Value;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum PromptRequestState {
    Active,
    Completed,
    Broken,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptRequestRow {
    session_id: SessionId,
    request_id: RequestId,
    // Derived preview of the submitted prompt for dashboards that want a
    // one-line summary without rehydrating chunk history.
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    state: PromptRequestState,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_reason: Option<StopReason>,
    started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChunkRowV2 {
    #[serde(rename = "chunkKey")]
    chunk_key: String,
    session_id: SessionId,
    request_id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<ToolCallId>,
    update: SessionUpdate,
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
    pending_session_creates: HashSet<String>,
    prompts_by_request: HashMap<String, PromptRequestRef>,
    prompt_rows: HashMap<String, PromptRequestRow>,
    active_prompts_by_session: HashMap<String, PromptRequestRef>,
    chunk_ordinals: HashMap<String, i64>,
}

pub struct StateProjector {
    correlation: TraceCorrelationState,
    supports_load_session: bool,
}

impl StateProjector {
    pub fn new() -> Self {
        Self {
            correlation: TraceCorrelationState::default(),
            supports_load_session: false,
        }
    }

    pub fn initial_events(&self) -> Vec<StateChange> {
        Vec::new()
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
                let Some(request_id) = request_id_from_json_value(&req.id) else {
                    return Vec::new();
                };
                self.correlation
                    .pending_initialize
                    .insert(request_id_key(&request_id));
                Vec::new()
            }
            "session/new" => {
                if !is_canonical_client_request(req) {
                    return Vec::new();
                }
                let Some(request_id) = request_id_from_json_value(&req.id) else {
                    return Vec::new();
                };
                self.correlation
                    .pending_session_creates
                    .insert(request_id_key(&request_id));
                Vec::new()
            }
            "session/prompt" => {
                if !is_canonical_client_request(req) {
                    return Vec::new();
                }
                let Some(request_id) = request_id_from_json_value(&req.id) else {
                    return Vec::new();
                };
                let Some(session_id) = session_id_from_params(&req.params) else {
                    return Vec::new();
                };

                let prompt_ref = PromptRequestRef {
                    session_id,
                    request_id,
                };
                let prompt_key = prompt_request_key(&prompt_ref);
                let prompt_row = PromptRequestRow {
                    session_id: prompt_ref.session_id.clone(),
                    request_id: prompt_ref.request_id.clone(),
                    text: prompt_text_preview(&req.params),
                    state: PromptRequestState::Active,
                    stop_reason: None,
                    started_at: now_ms(),
                    completed_at: None,
                };

                self.correlation.prompts_by_request.insert(
                    request_id_key(&prompt_ref.request_id),
                    prompt_ref.clone(),
                );
                self.correlation
                    .prompt_rows
                    .insert(prompt_key.clone(), prompt_row.clone());
                self.correlation.active_prompts_by_session.insert(
                    prompt_ref.session_id.to_string(),
                    prompt_ref,
                );

                vec![
                    state_change("prompt_request", &prompt_key, "insert", Some(&prompt_row)),
                ]
            }
            _ => Vec::new(),
        }
    }

    fn handle_response(&mut self, resp: &ResponseEvent) -> Vec<StateChange> {
        let Some(request_id) = request_id_from_json_value(&resp.id) else {
            return Vec::new();
        };
        let request_key = request_id_key(&request_id);

        if self.correlation.pending_initialize.remove(&request_key) {
            self.supports_load_session = resp
                .payload
                .get("agentCapabilities")
                .or_else(|| resp.payload.get("agent_capabilities"))
                .and_then(|caps| caps.get("loadSession").or_else(|| caps.get("load_session")))
                .and_then(Value::as_bool)
                .unwrap_or(false);
        }

        if self.correlation.pending_session_creates.remove(&request_key) {
            if resp.is_error {
                return Vec::new();
            }
            let Some(session_id) = resp
                .payload
                .get("sessionId")
                .or_else(|| resp.payload.get("session_id"))
                .and_then(Value::as_str)
                .map(|value| SessionId::from(value.to_string()))
            else {
                return Vec::new();
            };
            let now = now_ms();
            let session = SessionRecord {
                session_id: session_id.clone(),
                state: SessionStatus::Active,
                supports_load_session: self.supports_load_session,
                created_at: now,
                updated_at: now,
                last_seen_at: now,
            };

            return vec![
                state_change("session_v2", &session_id.to_string(), "insert", Some(&session)),
            ];
        }

        let Some(prompt_ref) = self.correlation.prompts_by_request.remove(&request_key) else {
            return Vec::new();
        };
        let prompt_key = prompt_request_key(&prompt_ref);
        let Some(mut prompt_row) = self.correlation.prompt_rows.get(&prompt_key).cloned() else {
            return Vec::new();
        };

        prompt_row.state = if resp.is_error {
            PromptRequestState::Broken
        } else {
            PromptRequestState::Completed
        };
        prompt_row.stop_reason = if resp.is_error {
            None
        } else {
            stop_reason_from_payload(&resp.payload)
        };
        prompt_row.completed_at = Some(now_ms());
        self.correlation
            .prompt_rows
            .insert(prompt_key.clone(), prompt_row.clone());
        let session_key = prompt_ref.session_id.to_string();
        self.correlation
            .active_prompts_by_session
            .remove(&session_key);
        self.correlation.chunk_ordinals.remove(&prompt_key);

        vec![
            state_change("prompt_request", &prompt_key, "update", Some(&prompt_row)),
        ]
    }

    fn handle_notification(&mut self, notif: &NotificationEvent) -> Vec<StateChange> {
        if notif.method != "session/update" || !is_canonical_session_update_notification(notif) {
            return Vec::new();
        }

        let Some(session_id) = notif
            .session
            .as_deref()
            .or_else(|| notif.params.get("sessionId").and_then(Value::as_str))
            .or_else(|| notif.params.get("session_id").and_then(Value::as_str))
            .map(|value| SessionId::from(value.to_string()))
        else {
            return Vec::new();
        };
        let session_key = session_id.to_string();
        let Some(prompt_ref) = self
            .correlation
            .active_prompts_by_session
            .get(&session_key)
            .cloned()
        else {
            return Vec::new();
        };

        let Some(update) = session_update_from_notification(notif.params.get("update")) else {
            return Vec::new();
        };

        let prompt_key = prompt_request_key(&prompt_ref);
        let order = self
            .correlation
            .chunk_ordinals
            .entry(prompt_key.clone())
            .or_insert(0);
        let current_order = *order;
        *order += 1;
        let created_at = now_ms();
        let tool_call_id = tool_call_id_from_session_update(&update);
        let chunk_key = chunk_row_key(&prompt_key, tool_call_id.as_ref(), current_order);
        let chunk_v2 = ChunkRowV2 {
            chunk_key: chunk_key.clone(),
            session_id: prompt_ref.session_id.clone(),
            request_id: prompt_ref.request_id.clone(),
            tool_call_id: tool_call_id.clone(),
            update: update.clone(),
            created_at,
        };
        vec![state_change("chunk_v2", &chunk_key, "insert", Some(&chunk_v2))]
    }
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

fn session_id_from_params(params: &Value) -> Option<SessionId> {
    params
        .get("sessionId")
        .or_else(|| params.get("session_id"))
        .and_then(Value::as_str)
        .map(|value| SessionId::from(value.to_string()))
}

fn prompt_text_preview(params: &Value) -> Option<String> {
    params
        .get("prompt")
        .and_then(Value::as_array)
        .and_then(|blocks| first_text_block(blocks))
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

fn request_id_from_json_value(value: &Value) -> Option<RequestId> {
    match serde_json::from_value(value.clone()) {
        Ok(request_id) => Some(request_id),
        Err(error) => {
            tracing::error!(
                ?value,
                ?error,
                "state projector dropped event with malformed canonical JSON-RPC request id"
            );
            None
        }
    }
}

fn request_id_key(request_id: &RequestId) -> String {
    match request_id {
        RequestId::Null => "null".to_string(),
        RequestId::Number(number) => number.to_string(),
        RequestId::Str(text) => text.clone(),
    }
}

fn prompt_request_key(prompt_ref: &PromptRequestRef) -> String {
    format!(
        "{}:{}",
        prompt_ref.session_id,
        request_id_key(&prompt_ref.request_id)
    )
}

fn chunk_row_key(prompt_key: &str, tool_call_id: Option<&ToolCallId>, ordinal: i64) -> String {
    match tool_call_id {
        Some(tool_call_id) => format!("{prompt_key}:{tool_call_id}:{ordinal}"),
        None => format!("{prompt_key}:{ordinal}"),
    }
}

fn session_update_from_notification(update: Option<&Value>) -> Option<SessionUpdate> {
    let value = update?.clone();
    match serde_json::from_value(value.clone()) {
        Ok(update) => Some(update),
        Err(error) => {
            tracing::debug!(
                ?value,
                ?error,
                "state projector dropped session/update payload that was not a canonical ACP SessionUpdate"
            );
            None
        }
    }
}

fn tool_call_id_from_session_update(update: &SessionUpdate) -> Option<ToolCallId> {
    match update {
        SessionUpdate::ToolCall(tool_call) => Some(tool_call.tool_call_id.clone()),
        SessionUpdate::ToolCallUpdate(tool_call_update) => Some(tool_call_update.tool_call_id.clone()),
        _ => None,
    }
}

fn stop_reason_from_payload(payload: &Value) -> Option<StopReason> {
    payload
        .get("stopReason")
        .or_else(|| payload.get("stop_reason"))
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

fn is_client_endpoint(raw: &str) -> bool {
    raw.trim().eq_ignore_ascii_case("client")
}

fn is_agent_endpoint(raw: &str) -> bool {
    raw.trim().eq_ignore_ascii_case("agent")
}

fn is_proxy_zero_endpoint(raw: &str) -> bool {
    let value = raw.trim().to_ascii_lowercase();
    value == "proxy(0)" || value == "proxy:0"
}

fn is_canonical_client_request(req: &RequestEvent) -> bool {
    is_client_endpoint(&req.from) && (is_proxy_zero_endpoint(&req.to) || is_agent_endpoint(&req.to))
}

fn is_canonical_session_update_notification(notif: &NotificationEvent) -> bool {
    is_proxy_zero_endpoint(&notif.to) || is_client_endpoint(&notif.to)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::ZERO)
        .as_millis() as i64
}
