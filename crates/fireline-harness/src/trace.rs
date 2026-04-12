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
use fireline_session::{HostDescriptor, PersistedHostSpec};
use sacp_conductor::trace::{TraceEvent, WriteEvent};
use serde::Serialize;

use crate::state_projector::StateProjector;

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
        host_id: impl Into<String>,
        connection_id: impl Into<String>,
    ) -> Self {
        let host_id = host_id.into();
        Self::new_with_host_context(
            producer,
            host_id.clone(),
            host_id,
            "node:unknown",
            connection_id,
        )
    }

    pub fn new_with_host_context(
        producer: Producer,
        host_key: impl Into<String>,
        host_id: impl Into<String>,
        node_id: impl Into<String>,
        connection_id: impl Into<String>,
    ) -> Self {
        let projector = StateProjector::new(host_key, host_id, node_id, connection_id);
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
enum RuntimeInstanceStatus {
    Running,
    Stopped,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeInstanceRow {
    instance_id: String,
    #[serde(rename = "runtimeName")]
    host_name: String,
    status: RuntimeInstanceStatus,
    created_at: i64,
    updated_at: i64,
}

pub async fn emit_host_instance_started(
    producer: &Producer,
    host_id: &str,
    host_name: &str,
    created_at: i64,
) -> anyhow::Result<()> {
    producer.append_json(&runtime_instance_event(
        host_id,
        host_name,
        created_at,
        RuntimeInstanceStatus::Running,
        "insert",
    ));
    producer.flush().await?;
    Ok(())
}

pub async fn emit_host_instance_stopped(
    producer: &Producer,
    host_id: &str,
    host_name: &str,
    created_at: i64,
) -> anyhow::Result<()> {
    producer.append_json(&runtime_instance_event(
        host_id,
        host_name,
        created_at,
        RuntimeInstanceStatus::Stopped,
        "update",
    ));
    // Explicit flush is load-bearing for stream-as-truth: without it, the
    // stopped envelope can be lost when the runtime process exits before
    // the producer's buffered writes have propagated. That divergence was
    // caught by tests/host_index_agreement.rs::host_index_observes_
    // stopped_runtimes_on_the_shared_stream, which would otherwise see a
    // control-plane-stopped runtime still advertised as Running on the
    // shared state stream.
    producer.flush().await?;
    Ok(())
}

fn runtime_instance_event(
    host_id: &str,
    host_name: &str,
    created_at: i64,
    status: RuntimeInstanceStatus,
    operation: &'static str,
) -> serde_json::Value {
    serde_json::to_value(StateEnvelope {
        entity_type: "runtime_instance",
        key: host_id.to_string(),
        headers: StateHeaders { operation },
        value: RuntimeInstanceRow {
            instance_id: host_id.to_string(),
            host_name: host_name.to_string(),
            status,
            created_at,
            updated_at: if operation == "insert" {
                created_at
            } else {
                now_ms()
            },
        },
    })
    .expect("serialize runtime_instance envelope")
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::ZERO)
        .as_millis() as i64
}

pub async fn emit_host_spec_persisted(
    state_stream_url: &str,
    spec: &PersistedHostSpec,
) -> anyhow::Result<()> {
    if state_stream_url.is_empty() {
        return Ok(());
    }

    let client = DurableStreamsClient::new();
    let mut stream = client.stream(state_stream_url);
    stream.set_content_type("application/json");
    let producer = stream
        .producer(format!("runtime-spec-{}", spec.host_key))
        .content_type("application/json")
        .build();
    producer.append_json(&StateEnvelope {
        entity_type: "runtime_spec",
        key: spec.host_key.clone(),
        headers: StateHeaders {
            operation: "insert",
        },
        value: spec,
    });
    producer.flush().await?;
    Ok(())
}

pub async fn emit_host_endpoints_persisted(
    state_stream_url: &str,
    descriptor: &HostDescriptor,
) -> anyhow::Result<()> {
    if state_stream_url.is_empty() {
        return Ok(());
    }

    let client = DurableStreamsClient::new();
    let mut stream = client.stream(state_stream_url);
    stream.set_content_type("application/json");
    let producer = stream
        .producer(format!(
            "runtime-endpoints-{}-{}",
            descriptor.host_key,
            uuid::Uuid::new_v4()
        ))
        .content_type("application/json")
        .build();
    producer.append_json(&StateEnvelope {
        entity_type: "runtime_endpoints",
        key: descriptor.host_key.clone(),
        headers: StateHeaders {
            operation: "update",
        },
        value: descriptor,
    });
    producer.flush().await?;
    Ok(())
}
