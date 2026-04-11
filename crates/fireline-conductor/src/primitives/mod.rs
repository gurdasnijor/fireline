//! Fireline substrate primitives: `Host`, `Sandbox`, `Orchestrator`.
//!
//! This module names the three primitives the Fireline substrate is built
//! around, mirroring the TypeScript surface formalized in
//! [`docs/proposals/client-primitives.md`](../../../../docs/proposals/client-primitives.md)
//! and reconciled against the existing Rust runtime machinery in
//! [`docs/proposals/runtime-host-split.md`](../../../../docs/proposals/runtime-host-split.md) §7.
//!
//! - [`Host`] — runs agent sessions. Owns session lifecycle and exposes
//!   the idempotent, retry-safe `wake` verb. Fireline's own
//!   [`crate::runtime::RuntimeHost`] is the first [`Host`] satisfier.
//! - [`Sandbox`] — runs a single tool call in isolation inside a running
//!   session (Anthropic's "Sandbox" primitive). Distinct from [`Host`];
//!   a [`Host`] can delegate tool execution to a [`Sandbox`], or bring
//!   its own. Microsandbox is slated as the first non-trivial satisfier.
//! - [`Orchestrator`] — substrate-agnostic wake loop that drives one or
//!   more [`Host`]s via a shared handler surface. Indifferent to which
//!   [`Host`] is in use; only knows how to call back into
//!   [`Host::wake`] with retry.
//!
//! These traits are additive. Existing call sites in
//! `fireline-control-plane`, `src/main.rs`, `src/bootstrap.rs`, and the
//! integration test suite continue to use [`crate::runtime::RuntimeHost`]
//! and the provider / launcher / registry types directly — this module
//! layers a common vocabulary on top of that code without changing any
//! of it. Future tiers (per §7.5 of the split proposal) layer additional
//! satisfiers (`MicrosandboxSandbox`, `ClaudeHost`, …) against the same
//! three traits.

pub mod host;
pub mod orchestration;
pub mod sandbox;

pub use host::{
    FIRELINE_HOST_KIND, FirelineHost, Host, SessionHandle, SessionSpec, SessionStatus, WakeOutcome,
};
pub use orchestration::Orchestrator;
pub use sandbox::{Sandbox, SandboxHandle, ToolCall, ToolCallResult};
