//! # fireline-components
//!
//! Prebuilt topology components for Fireline runtimes.
//!
//! Each submodule here contains a `ConnectTo<Conductor>` proxy, a
//! `WriteEvent` tracer, or an MCP bridge component that can be wired
//! into the conductor chain to enable a cross-cutting concern.
//!
//! Today the crate contains:
//!
//! - [`peer`] — cross-agent peer calls over ACP with in-band lineage
//!   propagation and child-session edge emission
//! - [`audit`] — a durable-stream audit tracer (SKETCH)
//! - [`context`] — inbound prompt context injection proxy (SKETCH)
//! - [`approval`] — policy-driven approval gate proxy (SKETCH)
//! - [`budget`] — per-session token / tool-call / duration budget gate (SKETCH)
//! - [`smithery`] — bridge that injects MCP servers hosted on Smithery (SKETCH)
//!
//! Components marked **SKETCH** compile and expose the right shapes but
//! their request-interception logic is TODO-marked pending a design
//! review on `ComponentRegistry` / `ComponentContext`. None of the
//! sketches are wired into the default conductor chain yet; the binary
//! crate continues to hand-wire [`peer::PeerComponent`] as before.

#![forbid(unsafe_code)]

pub mod peer;

pub mod audit;
pub mod context;
pub mod approval;
pub mod budget;
pub mod smithery;

// Backwards-compatible re-exports that match the old `fireline_peer`
// public surface. Consumers currently use `fireline_peer::PeerComponent`
// / `fireline_peer::Directory` / `fireline_peer::directory::Peer` /
// `fireline_peer::lookup::*` — the same paths work with
// `fireline_components::` after a simple crate-name substitution.
pub use peer::PeerComponent;
pub use peer::directory::{self, Directory};
pub use peer::lookup;
