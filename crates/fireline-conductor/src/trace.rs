//! Durable stream trace ingest.
//!
//! [`DurableStreamTracer`] implements [`sacp_conductor::trace::WriteEvent`] and
//! is the destination for `ConductorImpl::trace_to(...)`.
//!
//! The tracer itself is intentionally thin:
//!
//! - it receives ACP trace events
//! - forwards them into the Fireline state projector
//! - appends the resulting `STATE-PROTOCOL` changes to the durable stream
//!
//! The correlation and state-row projection logic lives in
//! [`crate::state_projector`], not here.

use std::io;

use durable_streams::Producer;
use sacp_conductor::trace::{TraceEvent, WriteEvent};

use crate::state_projector::{StateProjector, runtime_instance_started, runtime_instance_stopped};

pub struct DurableStreamTracer {
    producer: Producer,
    projector: StateProjector,
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
        )
    }

    pub fn new_with_runtime_context(
        producer: Producer,
        runtime_key: impl Into<String>,
        runtime_id: impl Into<String>,
        node_id: impl Into<String>,
        logical_connection_id: impl Into<String>,
    ) -> Self {
        let projector =
            StateProjector::new(runtime_key, runtime_id, node_id, logical_connection_id);
        for event in projector.initial_events() {
            producer.append_json(&event);
        }
        Self {
            producer,
            projector,
        }
    }
}

impl WriteEvent for DurableStreamTracer {
    fn write_event(&mut self, event: &TraceEvent) -> io::Result<()> {
        for state_change in self.projector.project_trace_event(event) {
            self.producer.append_json(&state_change);
        }
        Ok(())
    }
}

pub fn emit_runtime_instance_started(
    producer: &Producer,
    runtime_id: &str,
    runtime_name: &str,
    created_at: i64,
) {
    producer.append_json(&runtime_instance_started(
        runtime_id,
        runtime_name,
        created_at,
    ));
}

pub fn emit_runtime_instance_stopped(
    producer: &Producer,
    runtime_id: &str,
    runtime_name: &str,
    created_at: i64,
) {
    producer.append_json(&runtime_instance_stopped(
        runtime_id,
        runtime_name,
        created_at,
    ));
}
