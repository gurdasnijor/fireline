//! Fireline agents CLI — installs ACP agents from the public registry.

use anyhow::Result;
use clap::{Parser, Subcommand};
use fireline_tools::agent_catalog::install_agent_by_id;

#[derive(Debug, Parser)]
#[command(name = "fireline-agents")]
#[command(about = "Install ACP agents from the public registry", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Install an ACP agent by registry id.
    Add {
        /// Registry id, for example `pi-acp` or `claude-acp`.
        id: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Add { id } => {
            let installed_path = install_agent_by_id(&id).await?;
            println!("{}", installed_path.display());
            Ok(())
        }
    }
}
