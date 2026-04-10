//! Durable stream trace writer.
//!
//! [`DurableStreamTracer`] implements [`sacp_conductor::trace::WriteEvent`]
//! and is the destination for `ConductorImpl::trace_to(...)`. It
//! appends every observed [`sacp_conductor::trace::TraceEvent`] to a
//! durable stream as a JSON record with two extension fields:
//! `runtimeId` and `observedAtMs`.
//!
//! This is **pure observation**. The writer cannot modify messages,
//! does not maintain a correlator state machine, and does not
//! perform any message-level extension stamping. Anything that needs
//! to actively participate in the message flow lives in a
//! [`sacp::component::Component<sacp::ProxyToConductor>`] elsewhere
//! (e.g. `fireline-peer::PeerComponent` for cross-agent calls and
//! lineage propagation).
//!
//! The schema for the JSON record this writer produces is owned by
//! the TypeScript side at `packages/state/src/schema.ts`. The Rust
//! conformance test (`tests/schema_conformance.rs`) validates that
//! every record this writer emits matches that schema.

use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use durable_streams::Producer;
use sacp_conductor::trace::{TraceEvent, WriteEvent};
use serde::Serialize;

#[derive(Clone)]
pub struct DurableStreamTracer {
    producer: Producer,
    runtime_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TraceRecord<'a> {
    event: &'a TraceEvent,
    runtime_id: &'a str,
    observed_at_ms: u64,
}

impl DurableStreamTracer {
    pub fn new(producer: Producer, runtime_id: impl Into<String>) -> Self {
        Self {
            producer,
            runtime_id: runtime_id.into(),
        }
    }
}

impl WriteEvent for DurableStreamTracer {
    fn write_event(&mut self, event: &TraceEvent) -> io::Result<()> {
        let record = TraceRecord {
            event,
            runtime_id: &self.runtime_id,
            observed_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };

        self.producer.append_json(&record);
        Ok(())
    }
}
