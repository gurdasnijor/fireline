//! ACP agent catalog client.
//!
//! Fetches and caches the agent catalog from
//! <https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json>
//! to resolve agent IDs (e.g. `claude-acp`, `gemini`) to runnable
//! commands for the current platform.
//!
//! This is a CLI helper used by the `fireline-agents` binary's
//! `add` subcommand. It's not a runtime concern — the conductor
//! itself doesn't need this; only the CLI flow that lets users
//! configure which agents are available.
//!
//! Analogous to agent-os's [`registry/`](https://github.com/rivet-dev/agent-os/tree/main/registry)
//! directory, except we fetch from the central ACP registry rather
//! than maintaining our own catalog.

// TODO: implement agent catalog client
//
// Target shape:
//
// ```rust,ignore
// use serde::Deserialize;
//
// const REGISTRY_URL: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";
//
// #[derive(Debug, Deserialize)]
// pub struct AgentCatalog {
//     pub agents: Vec<RemoteAgent>,
// }
//
// #[derive(Debug, Deserialize)]
// pub struct RemoteAgent {
//     pub id: String,
//     pub name: String,
//     pub description: String,
//     pub distribution: Distribution,
// }
//
// pub async fn fetch_catalog() -> anyhow::Result<AgentCatalog>;
// pub fn resolve_command(agent: &RemoteAgent) -> anyhow::Result<Vec<String>>;
// ```
