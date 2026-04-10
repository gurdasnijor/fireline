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

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Peer {
    pub runtime_id: String,
    pub agent_name: String,
    pub acp_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_stream_url: Option<String>,
    pub registered_at_ms: i64,
}

#[derive(Clone, Debug)]
pub struct Directory {
    path: PathBuf,
}

impl Directory {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create peer directory parent {}", parent.display()))?;
        }
        if !path.exists() {
            fs::write(&path, "").with_context(|| format!("initialize {}", path.display()))?;
        }
        Ok(Self { path })
    }

    pub fn default_path() -> Result<PathBuf> {
        let base = dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .ok_or_else(|| anyhow!("resolve local data directory"))?;
        Ok(base.join("fireline").join("peers.toml"))
    }

    pub fn list(&self) -> Result<Vec<Peer>> {
        read_peers(&self.path)
    }

    pub fn register(&self, peer: Peer) -> Result<()> {
        let mut peers = self.list()?;
        peers.retain(|existing| existing.runtime_id != peer.runtime_id);
        peers.push(peer);
        write_peers(&self.path, &peers)
    }

    pub fn unregister(&self, runtime_id: &str) -> Result<()> {
        let mut peers = self.list()?;
        peers.retain(|peer| peer.runtime_id != runtime_id);
        write_peers(&self.path, &peers)
    }

    pub fn lookup(&self, agent_name: &str) -> Result<Option<Peer>> {
        Ok(self
            .list()?
            .into_iter()
            .find(|peer| peer.agent_name == agent_name))
    }
}

fn read_peers(path: &Path) -> Result<Vec<Peer>> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let file: PeerDirectoryFile =
        toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    Ok(file.peers)
}

fn write_peers(path: &Path, peers: &[Peer]) -> Result<()> {
    let raw = toml::to_string(&PeerDirectoryFile {
        peers: peers.to_vec(),
    })
    .context("serialize peers.toml")?;
    fs::write(path, raw).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct PeerDirectoryFile {
    #[serde(default)]
    peers: Vec<Peer>,
}
