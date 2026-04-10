//! axum route handlers for the Fireline binary.
//!
//! - [`acp`] — the `/acp` WebSocket route that mounts
//!   `fireline_conductor::transports::websocket::handle_upgrade`
//! - [`files`] — the `/api/v1/files/*` REST endpoints for the
//!   filesystem helper API; reads connection lookup files from the
//!   `crate::connections` module

pub mod acp;
pub mod files;
