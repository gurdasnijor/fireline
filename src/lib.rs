//! # fireline (binary internal modules)
//!
//! These modules are the binary's internal wiring. They're not
//! published as a library and aren't intended for external consumers.
//! The public substrate lives in `fireline-conductor` and
//! `fireline-peer`; everything in this crate is process-level glue:
//!
//! - [`bootstrap`] — composes the axum Router, embedded stream
//!   server, conductor builder, and connection lookup machinery into
//!   a running process
//! - [`routes`] — axum route handlers for the ACP WebSocket and
//!   filesystem helper API
//! - [`connections`] — connection lookup file management (the
//!   `{id}.toml` files written at session creation)
//! - [`webhook`] — outbound state-derived sink that subscribes to
//!   the durable stream via `durable-streams` client-rust and
//!   dispatches HTTP webhooks on transitions
//! - [`stream_host`] — embeds `durable-streams-server` in the same
//!   process as the conductor
//! - [`agent_catalog`] — ACP registry client used by the
//!   `fireline-agents` CLI to resolve agent IDs to runnable commands

pub mod agent_catalog;
pub mod bootstrap;
pub mod connections;
pub mod routes;
pub mod runtime_host;
pub mod runtime_registry;
pub mod stream_host;
pub mod webhook;
