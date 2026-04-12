//! Fireline agents CLI — installs ACP agents from the public registry.

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use fireline_tools::agent_catalog::{
    AgentCatalogClient, InstallPlan, LauncherKind, RemoteAgent, install_agent_dir,
    install_bin_dir, installed_command_path,
};
use reqwest::Client as HttpClient;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::fs;
use uuid::Uuid;

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
        Commands::Add { id } => add_agent(&id).await,
    }
}

async fn add_agent(id: &str) -> Result<()> {
    let client = AgentCatalogClient::new()?;
    let catalog = client.load_cached_or_fetch().await?;
    let agent = catalog
        .lookup(id)
        .ok_or_else(|| anyhow!("unknown ACP agent id '{id}'"))?;

    eprintln!("Installing {id}: {}", agent.description);
    let installed_path = install_agent(agent).await?;
    println!("{}", installed_path.display());
    Ok(())
}

async fn install_agent(agent: &RemoteAgent) -> Result<PathBuf> {
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
        InstallPlan::BinaryArchive { archive, command } => {
            install_binary_archive(agent, &archive, &command, &install_path).await?;
        }
    }

    Ok(install_path)
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
        bail!(
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
        bail!("unsupported ACP agent archive format: {}", archive_path.display());
    };

    if !status.success() {
        bail!(
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

async fn write_binary_wrapper(install_path: &Path, target_path: &Path, agent_id: &str) -> Result<()> {
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

fn normalized_relative_command(command: &str) -> PathBuf {
    let trimmed = command.strip_prefix("./").unwrap_or(command);
    PathBuf::from(trimmed)
}

#[cfg(not(windows))]
fn shell_escape_unix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
