//! Local peer directory.
//!
//! A file-backed directory of running Fireline instances on this
//! machine. Each entry records the runtime ID, the ACP endpoint URL,
//! the agent name, and the registration timestamp.
//!
//! Lifecycle:
//! - On Fireline startup: register self in the directory file
//! - On Fireline shutdown: unregister self
//! - On `list_peers` MCP tool call: read the current contents
//! - On `prompt_peer` MCP tool call: look up the target peer's endpoint
//!
//! The default location is
//! `~/.local/share/fireline/peers.toml` (resolved via the `dirs` crate).

// TODO: implement Directory
//
// Target shape:
//
// ```rust,ignore
// use std::path::PathBuf;
// use serde::{Serialize, Deserialize};
//
// #[derive(Clone, Debug, Serialize, Deserialize)]
// pub struct Peer {
//     pub runtime_id: String,
//     pub agent_name: String,
//     pub acp_url: String,
//     pub registered_at_ms: i64,
// }
//
// #[derive(Clone)]
// pub struct Directory {
//     path: PathBuf,
// }
//
// impl Directory {
//     pub fn load(path: impl Into<PathBuf>) -> anyhow::Result<Self>;
//     pub fn list(&self) -> anyhow::Result<Vec<Peer>>;
//     pub fn register(&self, peer: Peer) -> anyhow::Result<()>;
//     pub fn unregister(&self, runtime_id: &str) -> anyhow::Result<()>;
//     pub fn lookup(&self, agent_name: &str) -> anyhow::Result<Option<Peer>>;
// }
// ```
