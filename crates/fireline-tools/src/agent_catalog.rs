//! ACP agent catalog client.
//!
//! Fetches and caches the agent catalog from
//! <https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json>
//! so the install-time CLI can resolve ACP agent IDs (for example
//! `pi-acp` or `claude-acp`) to upstream-published distributions.
//!
//! This module is intentionally a CLI helper. The runtime conductor does
//! not depend on it.

use anyhow::{Context, Result, anyhow};
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const REGISTRY_URL: &str =
    "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";
const CACHE_RELATIVE_PATH: &str = "fireline/agent-catalog.json";
const INSTALL_ROOT_RELATIVE_PATH: &str = "fireline/agents";
const USER_AGENT: &str = "fireline-agent-catalog/0.0.1";

#[derive(Debug, Clone)]
pub struct AgentCatalogClient {
    http: HttpClient,
    registry_url: String,
    cache_path: PathBuf,
}

impl AgentCatalogClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: default_http_client()?,
            registry_url: REGISTRY_URL.to_string(),
            cache_path: default_cache_path()?,
        })
    }

    pub fn with_registry_url(mut self, registry_url: impl Into<String>) -> Self {
        self.registry_url = registry_url.into();
        self
    }

    pub fn with_cache_path(mut self, cache_path: impl Into<PathBuf>) -> Self {
        self.cache_path = cache_path.into();
        self
    }

    pub fn registry_url(&self) -> &str {
        &self.registry_url
    }

    pub fn cache_path(&self) -> &Path {
        &self.cache_path
    }

    pub async fn fetch(&self) -> Result<AgentCatalog> {
        fetch_catalog_with_client(&self.http, &self.registry_url).await
    }

    pub async fn fetch_and_cache(&self) -> Result<AgentCatalog> {
        let catalog = self.fetch().await?;
        catalog.write_cache(&self.cache_path)?;
        Ok(catalog)
    }

    pub fn load_cached(&self) -> Result<AgentCatalog> {
        AgentCatalog::load_cache(&self.cache_path)
    }

    pub async fn load_cached_or_fetch(&self) -> Result<AgentCatalog> {
        match self.load_cached() {
            Ok(catalog) => Ok(catalog),
            Err(_) => self.fetch_and_cache().await,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentCatalog {
    pub version: String,
    #[serde(default)]
    pub agents: Vec<RemoteAgent>,
    #[serde(default)]
    pub extensions: Vec<serde_json::Value>,
}

impl AgentCatalog {
    pub fn lookup(&self, id: &str) -> Option<&RemoteAgent> {
        self.agents.iter().find(|agent| agent.id == id)
    }

    pub fn load_cache(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read cached ACP registry from {}", path.display()))?;
        serde_json::from_str(&raw)
            .with_context(|| format!("parse cached ACP registry from {}", path.display()))
    }

    pub fn write_cache(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create registry cache dir {}", parent.display()))?;
        }
        let raw =
            serde_json::to_vec_pretty(self).context("serialize ACP registry cache for disk")?;
        std::fs::write(path, raw)
            .with_context(|| format!("write ACP registry cache to {}", path.display()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteAgent {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    pub license: String,
    pub icon: String,
    pub distribution: Distribution,
}

impl RemoteAgent {
    pub fn install_plan(&self) -> Result<InstallPlan> {
        if let Some(package) = &self.distribution.npx {
            return Ok(InstallPlan::PackageCommand {
                launcher: LauncherKind::Npx,
                package: package.package.clone(),
                args: package.args.clone(),
                env: package.env.clone(),
            });
        }

        if let Some(package) = &self.distribution.uvx {
            return Ok(InstallPlan::PackageCommand {
                launcher: LauncherKind::Uvx,
                package: package.package.clone(),
                args: package.args.clone(),
                env: package.env.clone(),
            });
        }

        let platform = current_binary_platform()?;
        let binary = self
            .distribution
            .binary
            .get(platform.as_str())
            .ok_or_else(|| anyhow!("agent '{}' has no binary distribution for {platform}", self.id))?;

        Ok(InstallPlan::BinaryArchive {
            archive: binary.archive.clone(),
            command: binary.cmd.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallPlan {
    PackageCommand {
        launcher: LauncherKind,
        package: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
    BinaryArchive {
        archive: String,
        command: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherKind {
    Npx,
    Uvx,
}

impl LauncherKind {
    pub fn executable(self) -> &'static str {
        match self {
            Self::Npx => "npx",
            Self::Uvx => "uvx",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Distribution {
    #[serde(default)]
    pub binary: BTreeMap<String, BinaryDistribution>,
    #[serde(default)]
    pub npx: Option<PackageDistribution>,
    #[serde(default)]
    pub uvx: Option<PackageDistribution>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BinaryDistribution {
    pub archive: String,
    pub cmd: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageDistribution {
    pub package: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

pub async fn fetch_catalog() -> Result<AgentCatalog> {
    AgentCatalogClient::new()?.fetch().await
}

pub async fn fetch_catalog_with_client(
    http: &HttpClient,
    registry_url: &str,
) -> Result<AgentCatalog> {
    let response = http
        .get(registry_url)
        .send()
        .await
        .with_context(|| format!("fetch ACP registry from {registry_url}"))?;
    let response = response
        .error_for_status()
        .with_context(|| format!("ACP registry returned error for {registry_url}"))?;
    response
        .json::<AgentCatalog>()
        .await
        .with_context(|| format!("deserialize ACP registry from {registry_url}"))
}

fn default_http_client() -> Result<HttpClient> {
    HttpClient::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| anyhow!("build ACP registry HTTP client: {error}"))
}

fn default_cache_path() -> Result<PathBuf> {
    let cache_root =
        dirs::cache_dir().ok_or_else(|| anyhow!("resolve OS cache directory for ACP registry"))?;
    Ok(cache_root.join(CACHE_RELATIVE_PATH))
}

pub fn install_root() -> Result<PathBuf> {
    let data_root =
        dirs::data_dir().ok_or_else(|| anyhow!("resolve OS data directory for ACP agents"))?;
    Ok(data_root.join(INSTALL_ROOT_RELATIVE_PATH))
}

pub fn install_bin_dir() -> Result<PathBuf> {
    Ok(install_root()?.join("bin"))
}

pub fn install_agent_dir(agent_id: &str) -> Result<PathBuf> {
    Ok(install_root()?.join(agent_id))
}

pub fn installed_command_path(agent_id: &str) -> Result<PathBuf> {
    Ok(install_bin_dir()?.join(installed_executable_name(agent_id)))
}

pub fn is_installed(agent_id: &str) -> Result<bool> {
    Ok(installed_command_path(agent_id)?.exists())
}

pub fn current_binary_platform() -> Result<String> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        "windows" => "windows",
        other => {
            return Err(anyhow!(
                "unsupported OS '{other}' for ACP registry binary resolution"
            ));
        }
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        other => {
            return Err(anyhow!(
                "unsupported architecture '{other}' for ACP registry binary resolution"
            ));
        }
    };
    Ok(format!("{os}-{arch}"))
}

fn installed_executable_name(agent_id: &str) -> String {
    #[cfg(windows)]
    {
        format!("{agent_id}.cmd")
    }
    #[cfg(not(windows))]
    {
        agent_id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use uuid::Uuid;

    const FIXTURE: &str = r#"{
  "version": "1.0.0",
  "agents": [
    {
      "id": "amp-acp",
      "name": "Amp",
      "version": "0.7.0",
      "description": "ACP wrapper for Amp",
      "repository": "https://github.com/example/amp-acp",
      "authors": ["amp"],
      "license": "Apache-2.0",
      "icon": "https://example.invalid/amp.svg",
      "distribution": {
        "binary": {
          "darwin-aarch64": {
            "archive": "https://example.invalid/amp-acp.tar.gz",
            "cmd": "./amp-acp"
          }
        }
      }
    },
    {
      "id": "claude-acp",
      "name": "Claude ACP",
      "version": "0.26.0",
      "description": "Anthropic ACP agent",
      "authors": ["acp"],
      "license": "MIT",
      "icon": "https://example.invalid/claude.svg",
      "distribution": {
        "npx": {
          "package": "@agentclientprotocol/claude-agent-acp@0.26.0"
        }
      }
    }
  ],
  "extensions": []
}"#;

    #[tokio::test]
    async fn fetches_and_deserializes_registry_fixture() {
        let registry_url = serve_registry_once(FIXTURE).await;
        let client = AgentCatalogClient::new()
            .unwrap()
            .with_registry_url(registry_url);

        let catalog = client.fetch().await.unwrap();

        assert_eq!(catalog.version, "1.0.0");
        assert_eq!(catalog.agents.len(), 2);
        assert_eq!(catalog.lookup("amp-acp").unwrap().name, "Amp");
        assert_eq!(
            catalog
                .lookup("claude-acp")
                .unwrap()
                .distribution
                .npx
                .as_ref()
                .unwrap()
                .package,
            "@agentclientprotocol/claude-agent-acp@0.26.0"
        );
    }

    #[test]
    fn writes_and_reads_cache_round_trip() {
        let cache_path = unique_test_path("agent-catalog-cache.json");
        let catalog: AgentCatalog = serde_json::from_str(FIXTURE).unwrap();

        catalog.write_cache(&cache_path).unwrap();
        let loaded = AgentCatalog::load_cache(&cache_path).unwrap();

        assert_eq!(loaded, catalog);
    }

    #[test]
    fn loads_cached_fixture_without_network() {
        let cache_path = unique_test_path("agent-catalog-fixture.json");
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        std::fs::write(&cache_path, FIXTURE).unwrap();

        let loaded = AgentCatalog::load_cache(&cache_path).unwrap();

        assert!(loaded.lookup("amp-acp").is_some());
        assert!(loaded.lookup("missing-agent").is_none());
    }

    #[test]
    fn resolves_install_plan_for_npx_agent() {
        let catalog: AgentCatalog = serde_json::from_str(FIXTURE).unwrap();
        let agent = catalog.lookup("claude-acp").unwrap();

        let plan = agent.install_plan().unwrap();

        assert_eq!(
            plan,
            InstallPlan::PackageCommand {
                launcher: LauncherKind::Npx,
                package: "@agentclientprotocol/claude-agent-acp@0.26.0".to_string(),
                args: vec![],
                env: BTreeMap::new(),
            }
        );
    }

    #[test]
    fn installed_command_path_uses_managed_bin_dir() {
        let path = installed_command_path("pi-acp").unwrap();
        let rendered = path.to_string_lossy();
        assert!(
            rendered.contains("fireline/agents/bin") || rendered.contains("fireline\\agents\\bin"),
            "managed install path should live under fireline/agents/bin, got {rendered}",
        );
    }

    async fn serve_registry_once(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0_u8; 4096];
            let _ = socket.read(&mut buf).await.unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        format!("http://{addr}/registry.json")
    }

    fn unique_test_path(file_name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("fireline-agent-catalog-{}", Uuid::new_v4()))
            .join(file_name)
    }
}
