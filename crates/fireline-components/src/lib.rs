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

#![forbid(unsafe_code)]

pub mod peer;

// Backwards-compatible re-exports that match the old `fireline_peer`
// public surface. Consumers currently use `fireline_peer::PeerComponent`
// / `fireline_peer::Directory` / `fireline_peer::directory::Peer` /
// `fireline_peer::lookup::*` — the same paths work with
// `fireline_components::` after a simple crate-name substitution.
pub use peer::PeerComponent;
pub use peer::directory::{self, Directory};
pub use peer::lookup;
