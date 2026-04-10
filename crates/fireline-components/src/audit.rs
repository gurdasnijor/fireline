//! Audit tracer — SKETCH.
//!
//! A [`WriteEvent`] implementation that appends structured audit
//! records to a dedicated durable stream. Unlike
//! `DurableStreamTracer`, this is a pure observer — it does not run
//! the state projector, it just writes one audit record per observed
//! ACP trace event.
//!
//! # SKETCH STATUS
//!
//! - Shape is correct: implements
//!   [`sacp_conductor::trace::WriteEvent`], writes via a
//!   [`durable_streams::Producer`].
//! - The `include_methods` filter is carried in config but not
//!   applied yet.
//! - Not wired into any `ConductorImpl::trace_to(...)` call in the
//!   binary. Consumers would attach it alongside
//!   `DurableStreamTracer` via a chained writer once the
//!   `ComponentRegistry` design converges.

use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use durable_streams::Producer;
use sacp_conductor::trace::{TraceEvent, WriteEvent};
use serde::Serialize;

/// Configuration for an [`AuditTracer`].
#[derive(Clone)]
pub struct AuditConfig {
    pub sink: AuditSink,
    /// Optional allow-list of method names. `None` means audit all
    /// methods. TODO: not yet applied inside `write_event`.
    pub include_methods: Option<Vec<String>>,
}

/// Where audit records go.
#[derive(Clone)]
pub enum AuditSink {
    /// Append-only durable stream, same substrate as the Fireline
    /// state stream but typically a different stream name with
    /// different retention.
    DurableStream { producer: Producer },
    // TODO: add Webhook, File, Stdout variants when a concrete
    // consumer appears and tells us what shape it wants.
}

/// A trace observer that serializes audit records to a sink.
pub struct AuditTracer {
    config: AuditConfig,
}

impl AuditTracer {
    pub fn new(config: AuditConfig) -> Self {
        Self { config }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditRecord {
    ts_ms: i64,
    event_kind: &'static str,
}

impl WriteEvent for AuditTracer {
    fn write_event(&mut self, event: &TraceEvent) -> io::Result<()> {
        // TraceEvent is `#[non_exhaustive]`; match each known variant
        // by tag only. A richer record (method, from, to) is TODO until
        // the exact inner-type field shapes are pinned in this crate.
        let event_kind = match event {
            TraceEvent::Request(_) => "request",
            TraceEvent::Response(_) => "response",
            TraceEvent::Notification(_) => "notification",
            _ => "unknown",
        };

        // TODO: apply self.config.include_methods filter once the
        // record carries the method name.

        let record = AuditRecord {
            ts_ms: now_ms(),
            event_kind,
        };

        match &self.config.sink {
            AuditSink::DurableStream { producer } => {
                producer.append_json(&record);
            }
        }

        Ok(())
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_record_serializes() {
        let record = AuditRecord {
            ts_ms: 1_000,
            event_kind: "request",
        };
        let value = serde_json::to_value(&record).expect("serialize audit record");
        assert_eq!(value["eventKind"], "request");
        assert_eq!(value["tsMs"], 1_000);
    }
}
