use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::{LazyLock, Mutex};

use fireline_acp_ids::{RequestId, SessionId, ToolCallId};
use fireline_tools::peer::PromptTraceContext;
use opentelemetry::trace::Status;
use sacp::schema::SessionUpdate;
use sacp_conductor::trace::{
    NotificationEvent, RequestEvent, ResponseEvent, TraceEvent, WriteEvent,
};
use serde_json::Value;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

type PromptKey = String;

static PROMPT_SPANS: LazyLock<Mutex<HashMap<PromptKey, Span>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static PROMPT_REQUEST_IDS: LazyLock<Mutex<HashMap<PromptKey, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static ACTIVE_PROMPTS_BY_SESSION: LazyLock<Mutex<HashMap<String, PromptKey>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static TOOL_SPANS: LazyLock<Mutex<HashMap<String, Span>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static APPROVAL_SPANS: LazyLock<Mutex<HashMap<PromptKey, Span>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Default)]
pub struct AgentPlaneTracer {
    pending_session_creates: HashSet<String>,
    prompt_keys_by_request: HashMap<String, PromptKey>,
}

impl AgentPlaneTracer {
    pub fn new() -> Self {
        Self::default()
    }

    fn handle_request(&mut self, req: &RequestEvent) {
        match req.method.as_str() {
            "session/new" if is_canonical_client_request(req) => {
                let Some(request_id) = request_id_from_json_value(&req.id) else {
                    return;
                };
                self.pending_session_creates
                    .insert(request_id_key(&request_id));
            }
            "session/prompt" if is_canonical_client_request(req) => {
                let Some(request_id) = request_id_from_json_value(&req.id) else {
                    return;
                };
                let Some(session_id) = session_id_from_params(&req.params) else {
                    return;
                };
                let prompt_key =
                    ensure_prompt_request_span(&session_id, &request_id, Some(req.method.as_str()));
                self.prompt_keys_by_request
                    .insert(request_id_key(&request_id), prompt_key);
            }
            _ => {}
        }
    }

    fn handle_response(&mut self, resp: &ResponseEvent) {
        let Some(request_id) = request_id_from_json_value(&resp.id) else {
            return;
        };
        let request_key = request_id_key(&request_id);

        if self.pending_session_creates.remove(&request_key) && !resp.is_error {
            let Some(session_id) = resp
                .payload
                .get("sessionId")
                .or_else(|| resp.payload.get("session_id"))
                .and_then(Value::as_str)
                .map(|value| SessionId::from(value.to_string()))
            else {
                return;
            };
            record_current_session_id(&session_id);
            let span = tracing::info_span!(
                "fireline.session.created",
                fireline.session_id = %session_id,
                rpc.system = "jsonrpc",
                rpc.method = "session/new",
            );
            let _enter = span.enter();
            return;
        }

        let Some(prompt_key) = self.prompt_keys_by_request.remove(&request_key) else {
            return;
        };
        finish_prompt_request_span(&prompt_key);
    }

    fn handle_notification(&mut self, notif: &NotificationEvent) {
        if notif.method != "session/update" || !is_canonical_session_update_notification(notif) {
            return;
        }

        let Some(session_id) = notif
            .session
            .as_deref()
            .or_else(|| notif.params.get("sessionId").and_then(Value::as_str))
            .or_else(|| notif.params.get("session_id").and_then(Value::as_str))
            .map(str::to_string)
        else {
            return;
        };
        let Some(prompt_key) = active_prompt_key_for_session(&session_id) else {
            return;
        };
        let Some(update) = notif.params.get("update") else {
            return;
        };

        match session_update_kind(update) {
            Some("tool_call") => start_tool_call_span(&prompt_key, &session_id, update),
            Some("tool_call_update") => update_tool_call_span(&prompt_key, &session_id, update),
            _ => {}
        }
    }
}

impl WriteEvent for AgentPlaneTracer {
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

pub(crate) fn ensure_prompt_request_span(
    session_id: &SessionId,
    request_id: &RequestId,
    method: Option<&str>,
) -> PromptKey {
    let prompt_key = prompt_key(session_id, request_id);
    record_current_session_id(session_id);
    let mut prompt_spans = PROMPT_SPANS.lock().expect("prompt span state poisoned");
    if !prompt_spans.contains_key(&prompt_key) {
        let span = match method {
            Some(method) => tracing::info_span!(
                "fireline.prompt.request",
                fireline.session_id = %session_id,
                fireline.request_id = %request_id_key(request_id),
                rpc.system = "jsonrpc",
                rpc.method = %method,
            ),
            None => tracing::info_span!(
                "fireline.prompt.request",
                fireline.session_id = %session_id,
                fireline.request_id = %request_id_key(request_id),
                rpc.system = "jsonrpc",
            ),
        };
        prompt_spans.insert(prompt_key.clone(), span);
    }
    PROMPT_REQUEST_IDS
        .lock()
        .expect("prompt span state poisoned")
        .insert(prompt_key.clone(), request_id_key(request_id));
    ACTIVE_PROMPTS_BY_SESSION
        .lock()
        .expect("prompt span state poisoned")
        .insert(session_id.to_string(), prompt_key.clone());
    prompt_key
}

pub(crate) fn start_approval_requested_span(
    session_id: &SessionId,
    request_id: &RequestId,
    policy_id: &str,
    reason: &str,
) {
    let prompt_key = ensure_prompt_request_span(session_id, request_id, None);
    let mut approval_spans = APPROVAL_SPANS.lock().expect("approval span state poisoned");
    if approval_spans.contains_key(&prompt_key) {
        return;
    }
    let Some(parent) = prompt_span(&prompt_key) else {
        return;
    };
    let span = tracing::info_span!(
        parent: &parent,
        "fireline.approval.requested",
        fireline.session_id = %session_id,
        fireline.request_id = %request_id_key(request_id),
        fireline.policy_id = %policy_id,
        fireline.reason = %reason,
    );
    approval_spans.insert(prompt_key, span);
}

pub(crate) fn emit_approval_resolved_span(
    session_id: &SessionId,
    request_id: &RequestId,
    allow: bool,
    resolved_by: &str,
) {
    let prompt_key = prompt_key(session_id, request_id);
    let approval_span = APPROVAL_SPANS
        .lock()
        .expect("approval span state poisoned")
        .remove(&prompt_key);
    let parent = approval_span.or_else(|| prompt_span(&prompt_key));
    let span = match parent {
        Some(parent) => tracing::info_span!(
            parent: &parent,
            "fireline.approval.resolved",
            fireline.session_id = %session_id,
            fireline.request_id = %request_id_key(request_id),
            fireline.allow = allow,
            fireline.resolved_by = %resolved_by,
        ),
        None => tracing::info_span!(
            "fireline.approval.resolved",
            fireline.session_id = %session_id,
            fireline.request_id = %request_id_key(request_id),
            fireline.allow = allow,
            fireline.resolved_by = %resolved_by,
        ),
    };
    let _enter = span.enter();
}

pub(crate) fn clear_approval_requested_span(session_id: &SessionId, request_id: &RequestId) {
    let prompt_key = prompt_key(session_id, request_id);
    APPROVAL_SPANS
        .lock()
        .expect("approval span state poisoned")
        .remove(&prompt_key);
}

fn prompt_span(prompt_key: &str) -> Option<Span> {
    PROMPT_SPANS
        .lock()
        .expect("prompt span state poisoned")
        .get(prompt_key)
        .cloned()
}

fn active_prompt_key_for_session(session_id: &str) -> Option<PromptKey> {
    ACTIVE_PROMPTS_BY_SESSION
        .lock()
        .expect("prompt span state poisoned")
        .get(session_id)
        .cloned()
}

pub(crate) fn active_prompt_trace_context_for_session(
    session_id: &str,
) -> Option<PromptTraceContext> {
    let prompt_key = active_prompt_key_for_session(session_id)?;
    let prompt_span = prompt_span(&prompt_key)?;
    let request_id = PROMPT_REQUEST_IDS
        .lock()
        .expect("prompt span state poisoned")
        .get(&prompt_key)
        .cloned()?;
    Some(PromptTraceContext {
        prompt_span,
        request_id,
    })
}

fn finish_prompt_request_span(prompt_key: &str) {
    ACTIVE_PROMPTS_BY_SESSION
        .lock()
        .expect("prompt span state poisoned")
        .retain(|_, active_key| active_key != prompt_key);
    PROMPT_REQUEST_IDS
        .lock()
        .expect("prompt span state poisoned")
        .remove(prompt_key);
    APPROVAL_SPANS
        .lock()
        .expect("approval span state poisoned")
        .remove(prompt_key);
    TOOL_SPANS
        .lock()
        .expect("tool span state poisoned")
        .retain(|tool_key, _| !tool_key.starts_with(prompt_key));
    PROMPT_SPANS
        .lock()
        .expect("prompt span state poisoned")
        .remove(prompt_key);
}

fn start_tool_call_span(prompt_key: &str, session_id: &str, update: &Value) {
    let Some(tool_call_id) = tool_call_id_from_update(update) else {
        return;
    };
    let tool_key = tool_span_key(prompt_key, &tool_call_id);
    let mut tool_spans = TOOL_SPANS.lock().expect("tool span state poisoned");
    if tool_spans.contains_key(&tool_key) {
        return;
    }
    let Some(parent) = prompt_span(prompt_key) else {
        return;
    };
    let tool_name = tool_name_from_update(update).unwrap_or_else(|| "unknown".to_string());
    let span = tracing::info_span!(
        parent: &parent,
        "fireline.tool.call",
        fireline.session_id = %session_id,
        fireline.tool_call_id = %tool_call_id,
        fireline.tool_name = %tool_name,
    );
    tool_spans.insert(tool_key, span);
}

fn update_tool_call_span(prompt_key: &str, session_id: &str, update: &Value) {
    let Some(tool_call_id) = tool_call_id_from_update(update) else {
        return;
    };
    let tool_key = tool_span_key(prompt_key, &tool_call_id);
    if !TOOL_SPANS
        .lock()
        .expect("tool span state poisoned")
        .contains_key(&tool_key)
    {
        start_tool_call_span(prompt_key, session_id, update);
    }
    let mut tool_spans = TOOL_SPANS.lock().expect("tool span state poisoned");
    let Some(span) = tool_spans.get(&tool_key).cloned() else {
        return;
    };
    if let Some(tool_name) = tool_name_from_update(update) {
        span.record("fireline.tool_name", tracing::field::display(tool_name));
    }
    let status = tool_call_status(update);
    if tool_call_failed(update) {
        span.set_status(Status::error(
            status.unwrap_or("tool call failed").to_string(),
        ));
    }
    if tool_call_terminal(status) {
        tool_spans.remove(&tool_key);
    }
}

fn tool_span_key(prompt_key: &str, tool_call_id: &ToolCallId) -> String {
    format!("{prompt_key}:{tool_call_id}")
}

fn prompt_key(session_id: &SessionId, request_id: &RequestId) -> PromptKey {
    format!("{}:{}", session_id, request_id_key(request_id))
}

fn record_current_session_id(session_id: &SessionId) {
    Span::current().record("fireline.session_id", tracing::field::display(session_id));
}

fn request_id_from_json_value(value: &Value) -> Option<RequestId> {
    serde_json::from_value(value.clone()).ok()
}

fn request_id_key(request_id: &RequestId) -> String {
    match request_id {
        RequestId::Null => "null".to_string(),
        RequestId::Number(number) => number.to_string(),
        RequestId::Str(text) => text.clone(),
    }
}

fn session_id_from_params(params: &Value) -> Option<SessionId> {
    params
        .get("sessionId")
        .or_else(|| params.get("session_id"))
        .and_then(Value::as_str)
        .map(|value| SessionId::from(value.to_string()))
}

fn tool_call_id_from_update(update: &Value) -> Option<ToolCallId> {
    update
        .get("toolCallId")
        .or_else(|| update.get("tool_call_id"))
        .and_then(Value::as_str)
        .map(|value| ToolCallId::from(value.to_string()))
        .or_else(
            || match serde_json::from_value::<SessionUpdate>(update.clone()).ok()? {
                SessionUpdate::ToolCall(tool_call) => Some(tool_call.tool_call_id),
                SessionUpdate::ToolCallUpdate(tool_call_update) => {
                    Some(tool_call_update.tool_call_id)
                }
                _ => None,
            },
        )
}

fn tool_name_from_update(update: &Value) -> Option<String> {
    [
        update.get("toolName"),
        update.get("tool_name"),
        update.get("name"),
        update.get("title"),
        update.get("kind"),
        update
            .get("fields")
            .and_then(|fields| fields.get("toolName")),
        update
            .get("fields")
            .and_then(|fields| fields.get("tool_name")),
        update.get("fields").and_then(|fields| fields.get("name")),
        update.get("fields").and_then(|fields| fields.get("title")),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_str)
    .map(str::to_string)
}

fn tool_call_status(update: &Value) -> Option<&str> {
    update
        .get("status")
        .or_else(|| update.get("fields").and_then(|fields| fields.get("status")))
        .and_then(Value::as_str)
}

fn tool_call_failed(update: &Value) -> bool {
    matches!(
        tool_call_status(update),
        Some("failed" | "cancelled" | "canceled" | "rejected" | "error")
    ) || update
        .get("isError")
        .or_else(|| update.get("is_error"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn tool_call_terminal(status: Option<&str>) -> bool {
    matches!(
        status,
        Some("completed" | "failed" | "cancelled" | "canceled" | "rejected" | "error")
    )
}

fn session_update_kind(update: &Value) -> Option<&str> {
    update
        .get("sessionUpdate")
        .or_else(|| update.get("session_update"))
        .and_then(Value::as_str)
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

#[cfg(test)]
fn reset_for_tests() {
    PROMPT_SPANS
        .lock()
        .expect("prompt span state poisoned")
        .clear();
    ACTIVE_PROMPTS_BY_SESSION
        .lock()
        .expect("prompt span state poisoned")
        .clear();
    TOOL_SPANS.lock().expect("tool span state poisoned").clear();
    APPROVAL_SPANS
        .lock()
        .expect("approval span state poisoned")
        .clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::Value as OtelValue;
    use opentelemetry::trace::{Status, TracerProvider as _};
    use opentelemetry_sdk::error::OTelSdkResult;
    use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter, Tracer};
    use std::sync::Arc;
    use tracing::Subscriber;
    use tracing::level_filters::LevelFilter;
    use tracing_subscriber::prelude::*;

    #[derive(Clone, Default, Debug)]
    struct TestExporter(Arc<Mutex<Vec<SpanData>>>);

    impl SpanExporter for TestExporter {
        async fn export(&self, mut batch: Vec<SpanData>) -> OTelSdkResult {
            self.0
                .lock()
                .expect("exporter state poisoned")
                .append(&mut batch);
            Ok(())
        }
    }

    fn test_tracer() -> (Tracer, SdkTracerProvider, TestExporter, impl Subscriber) {
        let exporter = TestExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let tracer = provider.tracer("fireline-harness-test");
        let subscriber = tracing_subscriber::registry().with(
            tracing_opentelemetry::layer()
                .with_tracer(tracer.clone())
                .with_filter(LevelFilter::INFO),
        );
        (tracer, provider, exporter, subscriber)
    }

    fn attr<'a>(span: &'a SpanData, key: &str) -> Option<&'a OtelValue> {
        span.attributes
            .iter()
            .find(|attribute| attribute.key.as_str() == key)
            .map(|attribute| &attribute.value)
    }

    fn request_event(value: serde_json::Value) -> RequestEvent {
        serde_json::from_value(value).expect("request event")
    }

    fn response_event(value: serde_json::Value) -> ResponseEvent {
        serde_json::from_value(value).expect("response event")
    }

    fn notification_event(value: serde_json::Value) -> NotificationEvent {
        serde_json::from_value(value).expect("notification event")
    }

    fn string_value(value: &str) -> OtelValue {
        value.to_string().into()
    }

    #[test]
    fn emits_required_spans_and_tool_failure_status() {
        reset_for_tests();
        let (_tracer, provider, exporter, subscriber) = test_tracer();

        tracing::subscriber::with_default(subscriber, || {
            let mut tracer = AgentPlaneTracer::new();

            tracer
                .write_event(&TraceEvent::Request(request_event(serde_json::json!({
                    "type": "request",
                    "ts": 0.0,
                    "protocol": "acp",
                    "from": "Client",
                    "to": "Agent",
                    "id": "session-new-1",
                    "method": "session/new",
                    "params": {"cwd": "/tmp"}
                }))))
                .expect("session/new request");
            tracer
                .write_event(&TraceEvent::Response(response_event(serde_json::json!({
                    "type": "response",
                    "ts": 0.0,
                    "from": "Agent",
                    "to": "Client",
                    "id": "session-new-1",
                    "is_error": false,
                    "payload": {"sessionId": "session-123"}
                }))))
                .expect("session/new response");

            tracer
                .write_event(&TraceEvent::Request(request_event(serde_json::json!({
                    "type": "request",
                    "ts": 0.0,
                    "protocol": "acp",
                    "from": "Client",
                    "to": "Agent",
                    "id": "prompt-1",
                    "method": "session/prompt",
                    "params": {
                        "sessionId": "session-123",
                        "prompt": [{"type": "text", "text": "please pause_here"}]
                    }
                }))))
                .expect("prompt request");

            let session_id = SessionId::from("session-123".to_string());
            let request_id = RequestId::Str("prompt-1".to_string());
            start_approval_requested_span(
                &session_id,
                &request_id,
                "prompt_contains:pause_here",
                "policy blocked the prompt",
            );
            emit_approval_resolved_span(&session_id, &request_id, false, "approval-test");

            tracer
                .write_event(&TraceEvent::Notification(notification_event(
                    serde_json::json!({
                        "type": "notification",
                        "ts": 0.0,
                        "protocol": "acp",
                        "from": "Agent",
                        "to": "Client",
                        "method": "session/update",
                        "params": {
                            "sessionId": "session-123",
                            "update": {
                                "sessionUpdate": "tool_call",
                                "toolCallId": "tool-1",
                                "title": "echo"
                            }
                        }
                    }),
                )))
                .expect("tool call notification");
            tracer
                .write_event(&TraceEvent::Notification(notification_event(
                    serde_json::json!({
                        "type": "notification",
                        "ts": 0.0,
                        "protocol": "acp",
                        "from": "Agent",
                        "to": "Client",
                        "method": "session/update",
                        "params": {
                            "sessionId": "session-123",
                            "update": {
                                "sessionUpdate": "tool_call_update",
                                "toolCallId": "tool-1",
                                "fields": {
                                    "status": "failed",
                                    "title": "echo"
                                }
                            }
                        }
                    }),
                )))
                .expect("tool call update notification");
            tracer
                .write_event(&TraceEvent::Response(response_event(serde_json::json!({
                    "type": "response",
                    "ts": 0.0,
                    "from": "Agent",
                    "to": "Client",
                    "id": "prompt-1",
                    "is_error": false,
                    "payload": {"stopReason": "end_turn"}
                }))))
                .expect("prompt response");
        });

        drop(provider);
        let spans = exporter.0.lock().expect("exporter state poisoned");

        let session_created = spans
            .iter()
            .find(|span| span.name == "fireline.session.created")
            .expect("session.created span");
        assert_eq!(
            attr(session_created, "fireline.session_id"),
            Some(&string_value("session-123"))
        );
        assert_eq!(
            attr(session_created, "rpc.system"),
            Some(&string_value("jsonrpc"))
        );

        let prompt_request = spans
            .iter()
            .find(|span| span.name == "fireline.prompt.request")
            .expect("prompt.request span");
        assert_eq!(
            attr(prompt_request, "fireline.session_id"),
            Some(&string_value("session-123"))
        );
        assert_eq!(
            attr(prompt_request, "fireline.request_id"),
            Some(&string_value("prompt-1"))
        );

        let approval_requested = spans
            .iter()
            .find(|span| span.name == "fireline.approval.requested")
            .expect("approval.requested span");
        assert_eq!(
            attr(approval_requested, "fireline.policy_id"),
            Some(&string_value("prompt_contains:pause_here"))
        );
        assert_eq!(
            attr(approval_requested, "fireline.reason"),
            Some(&string_value("policy blocked the prompt"))
        );
        assert_eq!(
            approval_requested.parent_span_id,
            prompt_request.span_context.span_id()
        );

        let approval_resolved = spans
            .iter()
            .find(|span| span.name == "fireline.approval.resolved")
            .expect("approval.resolved span");
        assert_eq!(
            attr(approval_resolved, "fireline.allow"),
            Some(&OtelValue::Bool(false))
        );
        assert_eq!(
            attr(approval_resolved, "fireline.resolved_by"),
            Some(&string_value("approval-test"))
        );
        assert_eq!(
            approval_resolved.parent_span_id,
            approval_requested.span_context.span_id()
        );

        let tool_call = spans
            .iter()
            .find(|span| span.name == "fireline.tool.call")
            .expect("tool.call span");
        assert_eq!(
            attr(tool_call, "fireline.tool_call_id"),
            Some(&string_value("tool-1"))
        );
        assert_eq!(
            attr(tool_call, "fireline.tool_name"),
            Some(&string_value("echo"))
        );
        assert_eq!(
            tool_call.parent_span_id,
            prompt_request.span_context.span_id()
        );
        assert_eq!(tool_call.status, Status::error("failed"));
    }
}
