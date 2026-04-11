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
//! - [`audit`] — a durable-stream audit tracer
//! - [`context`] — inbound prompt context injection proxy
//! - [`approval`] — policy-driven approval gate proxy (SKETCH)
//! - [`budget`] — per-session token / tool-call / duration budget gate (SKETCH)
//! - [`smithery`] — bridge that injects MCP servers hosted on Smithery (SKETCH)
//!
//! `audit` and `context` are fully implemented and intended for
//! topology/bootstrapping integration. Components still marked
//! **SKETCH** compile and expose the right shapes but are not yet
//! wired into Fireline's runtime topology path.

#![forbid(unsafe_code)]

pub mod peer;

pub mod approval;
pub mod audit;
pub mod budget;
pub mod context;
pub mod fs_backend;
pub mod smithery;
pub mod tools;

// Backwards-compatible re-exports that match the old `fireline_peer`
// public surface. Consumers currently use `fireline_peer::PeerComponent`
// / `fireline_peer::Directory` / `fireline_peer::directory::Peer` /
// `fireline_peer::lookup::*` — the same paths work with
// `fireline_components::` after a simple crate-name substitution.
pub use peer::PeerComponent;
pub use peer::directory::{self, Directory, LocalPeerDirectory, PeerRegistry};
pub use peer::lookup;
