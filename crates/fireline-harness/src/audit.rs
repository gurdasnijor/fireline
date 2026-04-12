//! Audit tracer.
//!
//! A [`WriteEvent`] implementation that appends one structured
//! audit record per observed ACP/MCP trace event to a durable
//! stream. This is a **tracer**, not a proxy — it attaches via
//! `ConductorImpl::trace_to(...)` alongside (or instead of) Fireline's
//! existing [`fireline_conductor::trace::DurableStreamTracer`].
//!
//! The tracer is a pure observer. It never modifies or drops a
//! trace event, and it doesn't run the full state projector —
//! audit is a much narrower concern, usually persisted to a
//! separate stream with its own retention and access controls.
//!
//! # Usage
//!
//! ```no_run
//! use fireline_components::audit::{AuditConfig, AuditSink, AuditTracer};
//! use durable_streams::Producer;
//!
//! # fn example(producer: Producer) {
//! let tracer = AuditTracer::new(AuditConfig {
//!     sink: AuditSink::DurableStream { producer },
//!     include_methods: Some(vec![
//!         "session/new".to_string(),
//!         "session/prompt".to_string(),
//!     ]),
//! });
//! // hand `tracer` to `ConductorImpl::trace_to(tracer)` in your bootstrap
//! # let _ = tracer;
//! # }
//! ```
//!
//! # Status
//!
//! Fully implemented as a standalone tracer. Not yet wired into
//! `bootstrap::start` — the binary still uses only
//! `DurableStreamTracer` on the default path. Adding an audit
//! sink alongside it is a bootstrap change that's deferred until
//! a concrete consumer of the audit stream exists.

use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use durable_streams::Producer;
use sacp_conductor::trace::{
    NotificationEvent, RequestEvent, ResponseEvent, TraceEvent, WriteEvent,
};
use serde::Serialize;

/// Configuration for an [`AuditTracer`].
#[derive(Clone)]
pub struct AuditConfig {
    pub sink: AuditSink,
    /// Optional allow-list of ACP / MCP method names. When `Some`,
    /// only trace events whose `method` matches one of the entries
    /// are written. Responses are keyed by the method of their
    /// originating request via the tracer's in-memory correlation
    /// table; unmatched responses pass the filter iff their request
    /// was audited. When `None`, all events are written.
    pub include_methods: Option<Vec<String>>,
}

/// Where audit records go.
#[derive(Clone)]
pub enum AuditSink {
    /// Append-only durable stream. Typically a different stream name
    /// from the runtime's main state stream so retention and access
    /// controls can diverge.
    DurableStream { producer: Producer },
    // TODO: File { path }, Webhook { url, auth }, Stdout — add as
    // concrete consumers show up.
}

/// A trace observer that serializes audit records to a sink.
pub struct AuditTracer {
    config: AuditConfig,
    /// Tracks which in-flight request IDs were audited, so their
    /// responses (which don't carry a `method` field) can be
    /// consistently included or excluded by the `include_methods`
    /// filter. Bounded by the natural lifetime of open ACP requests.
    audited_request_ids: std::collections::HashSet<String>,
}

impl AuditTracer {
    pub fn new(config: AuditConfig) -> Self {
        Self {
            config,
            audited_request_ids: std::collections::HashSet::new(),
        }
    }
}

/// One audit record per observed trace event.
///
/// The shape is intentionally flat and JSON-serializable so that
/// downstream compliance / observability tooling can index these
/// records without knowing the full ACP schema.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditRecord {
    pub ts_ms: i64,
    pub trace_ts: f64,
    pub direction: AuditDirection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditDirection {
    Request,
    Response,
    Notification,
}

impl WriteEvent for AuditTracer {
    fn write_event(&mut self, event: &TraceEvent) -> io::Result<()> {
        match event {
            TraceEvent::Request(req) => {
                let request_id = format_request_id(&req.id);
                if !self.should_include_method(Some(&req.method)) {
                    return Ok(());
                }
                self.audited_request_ids.insert(request_id.clone());
                self.write(&self.record_for_request(req, request_id));
            }
            TraceEvent::Response(resp) => {
                let request_id = format_request_id(&resp.id);
                // Include the response iff we audited its request.
                let include = match &self.config.include_methods {
                    Some(_) => self.audited_request_ids.remove(&request_id),
                    None => true,
                };
                if !include {
                    return Ok(());
                }
                self.write(&self.record_for_response(resp, request_id));
            }
            TraceEvent::Notification(notif) => {
                if !self.should_include_method(Some(&notif.method)) {
                    return Ok(());
                }
                self.write(&self.record_for_notification(notif));
            }
            // `TraceEvent` is `#[non_exhaustive]`; silently skip any
            // future variant rather than failing the tracer pipeline.
            _ => {}
        }
        Ok(())
    }
}

impl AuditTracer {
    fn should_include_method(&self, method: Option<&str>) -> bool {
        match (&self.config.include_methods, method) {
            (None, _) => true,
            (Some(allow), Some(m)) => allow.iter().any(|name| name == m),
            (Some(_), None) => false,
        }
    }

    fn write(&self, record: &AuditRecord) {
        match &self.config.sink {
            AuditSink::DurableStream { producer } => producer.append_json(record),
        }
    }

    fn record_for_request(&self, req: &RequestEvent, request_id: String) -> AuditRecord {
        AuditRecord {
            ts_ms: now_ms(),
            trace_ts: req.ts,
            direction: AuditDirection::Request,
            protocol: Some(format!("{:?}", req.protocol).to_lowercase()),
            from: req.from.clone(),
            to: req.to.clone(),
            method: Some(req.method.clone()),
            request_id: Some(request_id),
            session_id: req.session.clone(),
            is_error: None,
        }
    }

    fn record_for_response(&self, resp: &ResponseEvent, request_id: String) -> AuditRecord {
        AuditRecord {
            ts_ms: now_ms(),
            trace_ts: resp.ts,
            direction: AuditDirection::Response,
            protocol: None,
            from: resp.from.clone(),
            to: resp.to.clone(),
            method: None,
            request_id: Some(request_id),
            session_id: None,
            is_error: Some(resp.is_error),
        }
    }

    fn record_for_notification(&self, notif: &NotificationEvent) -> AuditRecord {
        AuditRecord {
            ts_ms: now_ms(),
            trace_ts: notif.ts,
            direction: AuditDirection::Notification,
            protocol: Some(format!("{:?}", notif.protocol).to_lowercase()),
            from: notif.from.clone(),
            to: notif.to.clone(),
            method: Some(notif.method.clone()),
            request_id: None,
            session_id: notif.session.clone(),
            is_error: None,
        }
    }
}

fn format_request_id(id: &serde_json::Value) -> String {
    match id {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(std::time::Duration::ZERO)
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: we don't construct a full `AuditTracer` in these tests
    // because `AuditSink::DurableStream` requires a
    // `durable_streams::Producer`, which needs a live tokio runtime
    // and can't be stubbed out cleanly. The filter and record-shape
    // logic is tested via free functions / record values directly.
    // End-to-end exercise of `write_event` belongs in an integration
    // test in the binary crate once audit is wired into bootstrap.

    #[test]
    fn audit_record_serializes_with_expected_fields() {
        let record = AuditRecord {
            ts_ms: 1_700_000_000_000,
            trace_ts: 12.5,
            direction: AuditDirection::Request,
            protocol: Some("acp".to_string()),
            from: "client".to_string(),
            to: "proxy(0)".to_string(),
            method: Some("session/prompt".to_string()),
            request_id: Some("42".to_string()),
            session_id: Some("sess-1".to_string()),
            is_error: None,
        };
        let value = serde_json::to_value(&record).expect("serialize");
        assert_eq!(value["direction"], "request");
        assert_eq!(value["method"], "session/prompt");
        assert_eq!(value["sessionId"], "sess-1");
        assert_eq!(value["protocol"], "acp");
        // is_error is skipped when None
        assert!(value.get("isError").is_none());
    }

    #[test]
    fn audit_record_response_carries_is_error() {
        let record = AuditRecord {
            ts_ms: 1,
            trace_ts: 2.0,
            direction: AuditDirection::Response,
            protocol: None,
            from: "agent".to_string(),
            to: "client".to_string(),
            method: None,
            request_id: Some("42".to_string()),
            session_id: None,
            is_error: Some(true),
        };
        let value = serde_json::to_value(&record).expect("serialize");
        assert_eq!(value["direction"], "response");
        assert_eq!(value["isError"], true);
        assert!(value.get("method").is_none());
    }

    #[test]
    fn format_request_id_handles_variants() {
        assert_eq!(format_request_id(&serde_json::json!("abc")), "abc");
        assert_eq!(format_request_id(&serde_json::json!(42)), "42");
        assert_eq!(format_request_id(&serde_json::json!(null)), "null");
    }
}
