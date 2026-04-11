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

use durable_streams::{Client as DurableStreamsClient, Producer};
use sacp_conductor::trace::{TraceEvent, WriteEvent};
use serde::Serialize;

use crate::runtime::PersistedRuntimeSpec;
use crate::state_projector::{StateProjector, runtime_instance_started, runtime_instance_stopped};

pub type BoxedTraceWriter = Box<dyn WriteEvent + Send>;

pub struct CompositeTraceWriter {
    writers: Vec<BoxedTraceWriter>,
}

impl CompositeTraceWriter {
    pub fn new(writers: Vec<BoxedTraceWriter>) -> Self {
        Self { writers }
    }
}

impl WriteEvent for CompositeTraceWriter {
    fn write_event(&mut self, event: &TraceEvent) -> io::Result<()> {
        for writer in &mut self.writers {
            writer.write_event(event)?;
        }
        Ok(())
    }
}

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

pub async fn emit_runtime_spec_persisted(
    state_stream_url: &str,
    spec: &PersistedRuntimeSpec,
) -> anyhow::Result<()> {
    if state_stream_url.is_empty() {
        return Ok(());
    }

    let client = DurableStreamsClient::new();
    let mut stream = client.stream(state_stream_url);
    stream.set_content_type("application/json");
    let producer = stream
        .producer(format!("runtime-spec-{}", spec.runtime_key))
        .content_type("application/json")
        .build();
    producer.append_json(&StateEnvelope {
        entity_type: "runtime_spec",
        key: spec.runtime_key.clone(),
        headers: StateHeaders {
            operation: "insert",
        },
        value: spec,
    });
    producer.flush().await?;
    Ok(())
}
