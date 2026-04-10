//! Fireline agents CLI — manages the local agent configuration.
//!
//! Subcommands:
//! - `add <id>` — fetch agent metadata from the ACP registry
//!   (via [`fireline::agent_catalog`]) and add it to the local
//!   `agents.toml` config
//! - `remove <name>` — remove an agent by name
//! - `list` — show currently configured agents
//! - `clear` — remove all configured agents
//!
//! This is a deployment-time tool, separate from the runtime
//! conductor binary.

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // TODO: implement fireline-agents CLI
    Ok(())
}
