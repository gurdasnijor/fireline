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
//! - [`error_codes`] — Fireline-specific ACP error codes and
//!   identifiers used in `_meta.fireline` responses
//! - [`load_coordinator`] — `session/load` coordination against the
//!   materialized session index
//! - [`session_index`] — in-memory materialization of durable `session`
//!   rows used for session lookup and future `session/load` coordination
//! - [`stream_host`] — embeds `durable-streams-server` in the same
//!   process as the conductor
//! - [`agent_catalog`] — ACP registry client used by the
//!   `fireline-agents` CLI to resolve agent IDs to runnable commands

pub mod agent_catalog;
pub mod bootstrap;
pub mod connections;
pub mod error_codes;
pub mod load_coordinator;
pub mod routes;
pub mod runtime_host;
pub mod runtime_registry;
pub mod session_index;
pub mod stream_host;
