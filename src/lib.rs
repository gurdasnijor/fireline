//! # fireline
//!
//! This crate is compiled both as the `fireline` binary and as a
//! library that process-level consumers (the workspace's binaries,
//! integration tests, and the control plane) compose against.
//!
//! The crate has a **small public surface** plus a larger set of
//! binary-internal modules that are NOT part of that surface. The
//! public surface is intentional: tightening it keeps "process glue"
//! from accidentally becoming substrate API that consumers grow to
//! depend on.
//!
//! ## Public surface
//!
//! - [`bootstrap`] — composes the axum Router, embedded stream
//!   server, conductor builder, and connection lookup machinery into
//!   a running process (`BootstrapConfig`, `BootstrapHandle`, `start`)
//! - [`orchestration`] — `resume`, `reconstruct_runtime_spec_from_log`,
//!   and `materialize_session_index`: the consumer-level composition
//!   helpers that recover a session across runtime death. Pure
//!   functions over Session + Sandbox state — no orchestration queue
//! - [`runtime_host`] — shared `RuntimeDescriptor`, `RuntimeStatus`,
//!   `Endpoint`, and `RuntimeProviderKind` types used by the control
//!   plane and every test harness
//! - [`runtime_registry`] — the `runtimes.toml` reader/writer used by
//!   the main binary and local-provider tests
//! - [`stream_host`] — embeds `durable-streams-server` in-process
//!   (`build_stream_router`, `StreamStorageConfig`)
//! - [`control_plane_client`] — HTTP client the main binary uses to
//!   register with a control plane and push heartbeats
//!
//! ## Binary-internal modules
//!
//! Everything else is process glue: request routing, connection
//! lookup files, error-code constants, topology wiring, and the
//! materializer/index projections that sit behind the crate-private
//! `bootstrap::AppState`. These are subject to change without notice
//! and must not be imported from outside this crate. If you find
//! yourself wanting to reach into one of them from a test or another
//! crate, that is a signal to promote the needed API into the public
//! surface above rather than broaden visibility by hand.
//!
//! For the physical/logical architecture of the workspace, see
//! `docs/architecture.md`.

// Public surface.
pub mod bootstrap;
pub mod control_plane_client;
pub mod orchestration;
pub mod runtime_host;
pub mod runtime_registry;
pub mod stream_host;

// Binary-internal modules. These must stay crate-private; see the
// module docstring for rationale.
pub(crate) mod active_turn_index;
pub(crate) mod agent_catalog;
pub(crate) mod child_session_edge;
pub(crate) mod connections;
pub(crate) mod control_plane_peer_registry;
pub(crate) mod error_codes;
pub(crate) mod load_coordinator;
pub(crate) mod routes;
pub(crate) mod runtime_materializer;
mod runtime_provider;
pub(crate) mod session_index;
pub(crate) mod topology;
