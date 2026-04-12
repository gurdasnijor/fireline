//! Local peer directory.
//!
//! This is a local-development bootstrap adapter, not part of Fireline's
//! durable or client-facing contract.
//!
//! A file-backed directory of running Fireline instances on one
//! machine. Each entry records the runtime ID, the ACP endpoint URL,
//! the agent name, and the registration timestamp.
//!
//! Lifecycle:
//! - On Fireline startup: register self in the local directory file
//! - On Fireline shutdown: unregister self
//! - On `list_peers` MCP tool call: read the current contents
//! - On `prompt_peer` MCP tool call: look up the target peer's endpoint
//!
//! The default location is
//! `~/.local/share/fireline/peers.toml` (resolved via the `dirs` crate).
//!
//! Important boundary:
//!
//! - this file is a local bootstrap convenience only
//! - it must not be treated as authoritative mesh state
//! - TypeScript clients and Flamecast should consume runtime descriptors,
//!   ACP endpoints, and durable state streams instead

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{Peer, PeerRegistry};

#[derive(Clone, Debug)]
pub struct LocalPeerDirectory {
    path: PathBuf,
}

impl LocalPeerDirectory {
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

#[async_trait]
impl PeerRegistry for LocalPeerDirectory {
    async fn list_peers(&self) -> Result<Vec<Peer>> {
        self.list()
    }

    async fn lookup_peer(&self, agent_name: &str) -> Result<Option<Peer>> {
        self.lookup(agent_name)
    }
}

pub type Directory = LocalPeerDirectory;

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
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&tmp, raw).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct PeerDirectoryFile {
    #[serde(default)]
    peers: Vec<Peer>,
}
