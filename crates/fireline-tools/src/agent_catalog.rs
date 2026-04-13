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
use std::process::Command;
use tokio::fs;
use uuid::Uuid;

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

pub async fn install_agent_by_id(id: &str) -> Result<PathBuf> {
    let client = AgentCatalogClient::new()?;
    if let Ok(cached) = client.load_cached() {
        if let Some(agent) = cached.lookup(id) {
            return install_agent(agent).await;
        }

        let refreshed = client.fetch_and_cache().await.with_context(|| {
            format!(
                "cached ACP registry does not contain '{id}' and refreshing the registry failed"
            )
        })?;
        let agent = refreshed
            .lookup(id)
            .ok_or_else(|| anyhow!("ACP registry does not contain an agent with id '{id}'"))?;
        return install_agent(agent).await;
    }

    let catalog = client
        .fetch_and_cache()
        .await
        .with_context(|| format!("fetch ACP registry while resolving '{id}'"))?;
    let agent = catalog
        .lookup(id)
        .ok_or_else(|| anyhow!("ACP registry does not contain an agent with id '{id}'"))?;
    install_agent(agent).await
}

pub async fn install_agent(agent: &RemoteAgent) -> Result<PathBuf> {
    fs::create_dir_all(install_bin_dir()?)
        .await
        .context("create managed ACP agent bin directory")?;

    let install_path = installed_command_path(&agent.id)?;
    match agent.install_plan()? {
        InstallPlan::PackageCommand {
            launcher,
            package,
            args,
            env,
        } => {
            match launcher {
                LauncherKind::Npx => {
                    install_npx_package(agent, &package, &args, &env, &install_path).await?;
                }
                LauncherKind::Uvx => {
                    write_wrapper(
                        &install_path,
                        launcher,
                        &package,
                        &args,
                        &env,
                        &agent.id,
                    )
                    .await?;
                }
            }
        }
        InstallPlan::BinaryArchive { archive, command } => {
            install_binary_archive(agent, &archive, &command, &install_path).await?;
        }
    }

    Ok(install_path)
}

pub async fn resolve_agent_launch_command(agent_command: Vec<String>) -> Result<Vec<String>> {
    let Some(token) = agent_command.first().cloned() else {
        return Ok(agent_command);
    };

    if agent_command.len() != 1 || is_explicit_path_like(&token) {
        return Ok(agent_command);
    }

    let installed_path = installed_command_path(&token)?;
    if installed_path.exists() {
        if needs_package_wrapper_upgrade(&installed_path)? {
            let upgraded = install_agent_by_id(&token).await.map_err(|error| {
                anyhow!(
                    "managed ACP agent '{token}' uses a legacy launcher wrapper and refresh failed: {error:#}"
                )
            })?;
            return Ok(vec![upgraded.to_string_lossy().into_owned()]);
        }
        return Ok(vec![installed_path.to_string_lossy().into_owned()]);
    }

    if command_exists_on_path(&token) {
        return Ok(agent_command);
    }

    let installed = install_agent_by_id(&token).await.map_err(|error| {
        anyhow!(
            "single-token agent command '{token}' was not found locally and ACP registry fallback failed: {error:#}\nHint: run `fireline-agents add {token}` manually or use an explicit command array."
        )
    })?;

    Ok(vec![installed.to_string_lossy().into_owned()])
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

async fn install_binary_archive(
    agent: &RemoteAgent,
    archive_url: &str,
    command: &str,
    install_path: &Path,
) -> Result<()> {
    let agent_dir = install_agent_dir(&agent.id)?;
    if agent_dir.exists() {
        std::fs::remove_dir_all(&agent_dir)
            .with_context(|| format!("remove prior install dir {}", agent_dir.display()))?;
    }
    fs::create_dir_all(&agent_dir)
        .await
        .with_context(|| format!("create install dir {}", agent_dir.display()))?;

    let http = HttpClient::builder()
        .user_agent("fireline-agents/0.0.1")
        .build()
        .context("build HTTP client for agent download")?;
    let bytes = http
        .get(archive_url)
        .send()
        .await
        .with_context(|| format!("download archive for {}", agent.id))?
        .error_for_status()
        .with_context(|| format!("agent archive request failed for {}", agent.id))?
        .bytes()
        .await
        .with_context(|| format!("read downloaded archive for {}", agent.id))?;

    let archive_path = std::env::temp_dir().join(format!(
        "fireline-agent-{}-{}{}",
        agent.id,
        Uuid::new_v4(),
        archive_extension(archive_url)
    ));
    fs::write(&archive_path, &bytes)
        .await
        .with_context(|| format!("write temp archive {}", archive_path.display()))?;

    extract_archive(&archive_path, &agent_dir)?;

    let target = normalized_relative_command(command);
    let target_path = agent_dir.join(&target);
    if !target_path.exists() {
        anyhow::bail!(
            "archive for '{}' extracted successfully but expected command '{}' was not found",
            agent.id,
            target_path.display()
        );
    }

    write_binary_wrapper(install_path, &target_path, &agent.id).await?;

    let _ = fs::remove_file(&archive_path).await;
    Ok(())
}

fn extract_archive(archive_path: &Path, install_dir: &Path) -> Result<()> {
    let archive = archive_path.to_string_lossy().to_string();
    let install_dir = install_dir.to_string_lossy().to_string();
    let status = if archive.ends_with(".tar.gz") || archive.ends_with(".tgz") {
        Command::new("tar")
            .args(["-xzf", &archive, "-C", &install_dir])
            .status()
            .context("spawn tar for ACP agent archive extraction")?
    } else if archive.ends_with(".zip") {
        Command::new("unzip")
            .args(["-o", &archive, "-d", &install_dir])
            .status()
            .context("spawn unzip for ACP agent archive extraction")?
    } else {
        anyhow::bail!(
            "unsupported ACP agent archive format: {}",
            archive_path.display()
        );
    };

    if !status.success() {
        anyhow::bail!(
            "archive extraction failed for {} with status {}",
            archive_path.display(),
            status
        );
    }

    Ok(())
}

async fn write_wrapper(
    install_path: &Path,
    launcher: LauncherKind,
    package: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    agent_id: &str,
) -> Result<()> {
    #[cfg(windows)]
    let script = build_windows_package_wrapper(launcher, package, args, env);
    #[cfg(not(windows))]
    let script = build_unix_package_wrapper(launcher, package, args, env);

    fs::write(install_path, script)
        .await
        .with_context(|| format!("write launcher wrapper for {agent_id}"))?;
    make_executable(install_path)?;
    Ok(())
}

async fn install_npx_package(
    agent: &RemoteAgent,
    package: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    install_path: &Path,
) -> Result<()> {
    let agent_dir = install_agent_dir(&agent.id)?;
    if agent_dir.exists() {
        std::fs::remove_dir_all(&agent_dir)
            .with_context(|| format!("remove prior install dir {}", agent_dir.display()))?;
    }
    fs::create_dir_all(&agent_dir)
        .await
        .with_context(|| format!("create install dir {}", agent_dir.display()))?;

    let status = Command::new("npm")
        .args(["install", "--silent", "--no-save", "--prefix"])
        .arg(&agent_dir)
        .arg(package)
        .status()
        .context("spawn npm install for ACP package agent")?;
    if !status.success() {
        anyhow::bail!("npm install failed for ACP package agent '{package}' with status {status}");
    }

    let package_name = normalized_package_name(package);
    let package_dir = installed_package_dir(&agent_dir, &package_name);
    let package_json = std::fs::read_to_string(package_dir.join("package.json"))
        .with_context(|| format!("read installed package manifest {}", package_dir.display()))?;
    let package_manifest: serde_json::Value =
        serde_json::from_str(&package_json).context("parse installed package manifest")?;
    let binary_name = installed_binary_name(&agent.id, &package_name, &package_manifest)?;
    let binary_path = installed_binary_path(&agent_dir, &binary_name);

    if !binary_path.exists() {
        anyhow::bail!(
            "installed ACP package binary '{}' not found at {}",
            binary_name,
            binary_path.display()
        );
    }

    write_env_binary_wrapper(install_path, &binary_path, args, env, &agent.id).await
}

async fn write_binary_wrapper(
    install_path: &Path,
    target_path: &Path,
    agent_id: &str,
) -> Result<()> {
    #[cfg(windows)]
    let script = format!("@echo off\r\n\"{}\" %*\r\n", target_path.display());
    #[cfg(not(windows))]
    let script = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nexec \"{}\" \"$@\"\n",
        target_path.display()
    );

    fs::write(install_path, script)
        .await
        .with_context(|| format!("write binary wrapper for {agent_id}"))?;
    make_executable(install_path)?;
    Ok(())
}

async fn write_env_binary_wrapper(
    install_path: &Path,
    target_path: &Path,
    args: &[String],
    env: &BTreeMap<String, String>,
    agent_id: &str,
) -> Result<()> {
    #[cfg(windows)]
    let script = build_windows_binary_wrapper(target_path, args, env);
    #[cfg(not(windows))]
    let script = build_unix_binary_wrapper(target_path, args, env);

    fs::write(install_path, script)
        .await
        .with_context(|| format!("write binary wrapper for {agent_id}"))?;
    make_executable(install_path)?;
    Ok(())
}

#[cfg(not(windows))]
fn build_unix_package_wrapper(
    launcher: LauncherKind,
    package: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> String {
    let mut script = String::from("#!/usr/bin/env bash\nset -euo pipefail\n");
    for (key, value) in env {
        script.push_str(&format!(
            "export {}={}\n",
            key,
            shell_escape_unix(value)
        ));
    }
    let extra_args = args
        .iter()
        .map(|arg| shell_escape_unix(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let yes = match launcher {
        LauncherKind::Npx => " -y",
        LauncherKind::Uvx => "",
    };
    let tail = if extra_args.is_empty() {
        String::new()
    } else {
        format!(" {extra_args}")
    };
    script.push_str(&format!(
        "exec {}{} {}{} \"$@\"\n",
        launcher.executable(),
        yes,
        shell_escape_unix(package),
        tail
    ));
    script
}

#[cfg(not(windows))]
fn build_unix_binary_wrapper(
    target_path: &Path,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> String {
    let mut script = String::from("#!/usr/bin/env bash\nset -euo pipefail\n");
    for (key, value) in env {
        script.push_str(&format!(
            "export {}={}\n",
            key,
            shell_escape_unix(value)
        ));
    }
    let extra_args = args
        .iter()
        .map(|arg| shell_escape_unix(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let tail = if extra_args.is_empty() {
        String::new()
    } else {
        format!(" {extra_args}")
    };
    script.push_str(&format!(
        "exec \"{}\"{} \"$@\"\n",
        target_path.display(),
        tail
    ));
    script
}

#[cfg(windows)]
fn build_windows_package_wrapper(
    launcher: LauncherKind,
    package: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> String {
    let mut script = String::from("@echo off\r\n");
    for (key, value) in env {
        script.push_str(&format!("set {}={}\r\n", key, value));
    }
    let extra_args = args.join(" ");
    let yes = match launcher {
        LauncherKind::Npx => " -y",
        LauncherKind::Uvx => "",
    };
    let tail = if extra_args.is_empty() {
        String::new()
    } else {
        format!(" {extra_args}")
    };
    script.push_str(&format!(
        "{}{} {}{} %*\r\n",
        launcher.executable(),
        yes,
        package,
        tail
    ));
    script
}

#[cfg(windows)]
fn build_windows_binary_wrapper(
    target_path: &Path,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> String {
    let mut script = String::from("@echo off\r\n");
    for (key, value) in env {
        script.push_str(&format!("set {}={}\r\n", key, value));
    }
    let extra_args = args.join(" ");
    let tail = if extra_args.is_empty() {
        String::new()
    } else {
        format!(" {extra_args}")
    };
    script.push_str(&format!("\"{}\"{} %*\r\n", target_path.display(), tail));
    script
}

#[cfg(not(windows))]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path)
        .with_context(|| format!("read wrapper metadata {}", path.display()))?;
    let mut perms = metadata.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("mark wrapper executable {}", path.display()))
}

#[cfg(windows)]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn archive_extension(url: &str) -> &'static str {
    if url.ends_with(".tar.gz") {
        ".tar.gz"
    } else if url.ends_with(".tgz") {
        ".tgz"
    } else if url.ends_with(".zip") {
        ".zip"
    } else {
        ".bin"
    }
}

fn normalized_package_name(package_spec: &str) -> String {
    if package_spec.starts_with('@') {
        match package_spec[1..].find('@') {
            Some(index) => package_spec[..index + 1].to_string(),
            None => package_spec.to_string(),
        }
    } else {
        package_spec
            .split_once('@')
            .map(|(name, _)| name.to_string())
            .unwrap_or_else(|| package_spec.to_string())
    }
}

fn installed_package_dir(agent_dir: &Path, package_name: &str) -> PathBuf {
    let mut path = agent_dir.join("node_modules");
    for segment in package_name.split('/') {
        path.push(segment);
    }
    path
}

fn installed_binary_path(agent_dir: &Path, binary_name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        return agent_dir
            .join("node_modules")
            .join(".bin")
            .join(format!("{binary_name}.cmd"));
    }
    #[cfg(not(windows))]
    {
        agent_dir.join("node_modules").join(".bin").join(binary_name)
    }
}

fn installed_binary_name(
    agent_id: &str,
    package_name: &str,
    package_manifest: &serde_json::Value,
) -> Result<String> {
    let default_name = package_name
        .rsplit('/')
        .next()
        .unwrap_or(package_name)
        .to_string();
    let Some(bin) = package_manifest.get("bin") else {
        return Ok(default_name);
    };

    if let Some(path) = bin.as_str() {
        if !path.is_empty() {
            return Ok(default_name);
        }
    }

    let Some(bin_map) = bin.as_object() else {
        return Ok(default_name);
    };

    if bin_map.len() == 1 {
        return Ok(bin_map.keys().next().expect("single bin key").to_string());
    }

    if bin_map.contains_key(agent_id) {
        return Ok(agent_id.to_string());
    }

    if bin_map.contains_key(&default_name) {
        return Ok(default_name);
    }

    anyhow::bail!(
        "installed ACP package '{}' exposes multiple binaries ({:?}); could not choose one for agent '{}'",
        package_name,
        bin_map.keys().collect::<Vec<_>>(),
        agent_id
    )
}

fn needs_package_wrapper_upgrade(path: &Path) -> Result<bool> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read installed ACP wrapper {}", path.display()))?;
    Ok(raw.contains("exec npx ") || raw.contains("exec uvx ") || raw.contains("\r\nnpx "))
}

fn normalized_relative_command(command: &str) -> PathBuf {
    let trimmed = command.strip_prefix("./").unwrap_or(command);
    PathBuf::from(trimmed)
}

fn is_explicit_path_like(command: &str) -> bool {
    let path = Path::new(command);
    path.is_absolute() || path.components().count() > 1
}

fn command_exists_on_path(command: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    let path_exts = windows_path_exts();

    std::env::split_paths(&paths).any(|dir| {
        if command_has_known_extension(command) {
            dir.join(command).is_file()
        } else {
            candidate_names(command, &path_exts)
                .into_iter()
                .any(|candidate| dir.join(candidate).is_file())
        }
    })
}

#[cfg(windows)]
fn windows_path_exts() -> Vec<String> {
    std::env::var("PATHEXT")
        .ok()
        .map(|value| value.split(';').map(|part| part.to_ascii_lowercase()).collect())
        .unwrap_or_else(|| vec![".exe".to_string(), ".cmd".to_string(), ".bat".to_string()])
}

#[cfg(not(windows))]
fn windows_path_exts() -> Vec<String> {
    Vec::new()
}

fn candidate_names(command: &str, path_exts: &[String]) -> Vec<String> {
    if path_exts.is_empty() {
        return vec![command.to_string()];
    }
    let mut names = Vec::with_capacity(path_exts.len() + 1);
    names.push(command.to_string());
    names.extend(path_exts.iter().map(|ext| format!("{command}{ext}")));
    names
}

fn command_has_known_extension(command: &str) -> bool {
    Path::new(command).extension().is_some()
}

#[cfg(not(windows))]
fn shell_escape_unix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
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

    #[tokio::test]
    async fn resolve_launch_command_leaves_multi_token_commands_untouched() {
        let command = vec!["node".to_string(), "agent.mjs".to_string()];
        assert_eq!(resolve_agent_launch_command(command.clone()).await.unwrap(), command);
    }

    #[tokio::test]
    async fn resolve_launch_command_leaves_explicit_paths_untouched() {
        let command = vec!["./pi-acp".to_string()];
        assert_eq!(resolve_agent_launch_command(command.clone()).await.unwrap(), command);
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
