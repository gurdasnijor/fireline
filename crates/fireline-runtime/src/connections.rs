//! Connection lookup file management.
//!
//! When a session is created, the bootstrap writes a small TOML file
//! at `~/.local/share/fireline/runtime/connections/{id}.toml` with the
//! connection's metadata: `runtime_id`, `cwd`, `started_at`, etc.
//!
//! When a session ends (or the binary shuts down), the file is
//! removed.
//!
//! The helper API (`crate::routes::files`) reads these files to
//! resolve `connection_id → cwd` without needing access to any
//! in-process state. This replaces the in-memory `StreamDb` lookup
//! pattern from earlier iterations of the codebase.

// TODO: implement connection lookup file management
//
// Target shape:
//
// ```rust,ignore
// use serde::{Serialize, Deserialize};
// use std::path::PathBuf;
//
// #[derive(Clone, Debug, Serialize, Deserialize)]
// pub struct ConnectionRecord {
//     pub connection_id: String,
//     pub runtime_id: String,
//     pub session_id: Option<String>,
//     pub cwd: Option<String>,
//     pub started_at_ms: i64,
// }
//
// pub fn write(record: &ConnectionRecord) -> anyhow::Result<()>;
// pub fn read(connection_id: &str) -> anyhow::Result<Option<ConnectionRecord>>;
// pub fn remove(connection_id: &str) -> anyhow::Result<()>;
//
// fn dir() -> anyhow::Result<PathBuf>;  // ~/.local/share/fireline/runtime/connections/
// ```
