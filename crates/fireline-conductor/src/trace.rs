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

// TODO: implement DurableStreamTracer
//
// Target shape:
//
// ```rust,ignore
// pub struct DurableStreamTracer {
//     producer: std::sync::Mutex<durable_streams::Producer>,
//     runtime_id: String,
// }
//
// impl DurableStreamTracer {
//     pub fn new(producer: durable_streams::Producer, runtime_id: impl Into<String>) -> Self;
// }
//
// impl sacp_conductor::trace::WriteEvent for DurableStreamTracer {
//     fn write_event(&mut self, event: &sacp_conductor::trace::TraceEvent) -> std::io::Result<()> {
//         let record = serde_json::json!({
//             "event": event,
//             "runtimeId": self.runtime_id,
//             "observedAtMs": now_ms(),
//         });
//         let producer = self.producer.lock().unwrap();
//         producer.append_json(&record);
//         Ok(())
//     }
// }
// ```
//
// See `docs/architecture.md` § "fireline-conductor" for the design intent.
