use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use fireline_conductor::runtime::{
    CreateRuntimeSpec, LocalRuntimeLauncher, ManagedRuntime, RuntimeDescriptor, RuntimeLaunch,
    RuntimeRegistry, RuntimeStatus,
};
use tokio::process::{Child, Command};

pub struct ChildProcessRuntimeLauncher {
    fireline_bin: PathBuf,
    runtime_registry: RuntimeRegistry,
    runtime_registry_path: PathBuf,
    default_peer_directory_path: Option<PathBuf>,
    startup_timeout: Duration,
    stop_timeout: Duration,
    poll_interval: Duration,
}

impl ChildProcessRuntimeLauncher {
    pub fn new(
        fireline_bin: PathBuf,
        runtime_registry: RuntimeRegistry,
        runtime_registry_path: PathBuf,
        default_peer_directory_path: Option<PathBuf>,
        startup_timeout: Duration,
        stop_timeout: Duration,
    ) -> Self {
        Self {
            fireline_bin,
            runtime_registry,
            runtime_registry_path,
            default_peer_directory_path,
            startup_timeout,
            stop_timeout,
            poll_interval: Duration::from_millis(100),
        }
    }

    async fn wait_for_runtime_ready(
        &self,
        runtime_key: &str,
        child: &mut Child,
    ) -> Result<RuntimeDescriptor> {
        let deadline = tokio::time::Instant::now() + self.startup_timeout;
        loop {
            if let Some(runtime) = self.runtime_registry.get(runtime_key)? {
                if runtime.status == RuntimeStatus::Ready {
                    return Ok(runtime);
                }
            }

            if let Some(status) = child.try_wait()? {
                return Err(anyhow!(
                    "fireline runtime exited before becoming ready: {status}"
                ));
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "timed out waiting for runtime '{runtime_key}' to become ready"
                ));
            }

            tokio::time::sleep(self.poll_interval).await;
        }
    }
}

#[async_trait]
impl LocalRuntimeLauncher for ChildProcessRuntimeLauncher {
    async fn start_local_runtime(
        &self,
        spec: CreateRuntimeSpec,
        runtime_key: String,
        node_id: String,
    ) -> Result<RuntimeLaunch> {
        let mut command = Command::new(&self.fireline_bin);
        command
            .arg("--host")
            .arg(spec.host.to_string())
            .arg("--port")
            .arg(spec.port.to_string())
            .arg("--name")
            .arg(&spec.name)
            .arg("--runtime-key")
            .arg(&runtime_key)
            .arg("--node-id")
            .arg(&node_id)
            .arg("--runtime-registry-path")
            .arg(&self.runtime_registry_path);

        if let Some(state_stream) = &spec.state_stream {
            command.arg("--state-stream").arg(state_stream);
        }

        if let Some(peer_directory_path) = spec
            .peer_directory_path
            .as_ref()
            .or(self.default_peer_directory_path.as_ref())
        {
            command
                .arg("--peer-directory-path")
                .arg(peer_directory_path);
        }

        if !spec.topology.components.is_empty() {
            command
                .arg("--topology-json")
                .arg(serde_json::to_string(&spec.topology).context("serialize topology")?);
        }

        command
            .arg("--")
            .args(&spec.agent_command)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let mut child = command
            .spawn()
            .with_context(|| format!("spawn fireline binary {}", self.fireline_bin.display()))?;

        let descriptor = match self.wait_for_runtime_ready(&runtime_key, &mut child).await {
            Ok(descriptor) => descriptor,
            Err(error) => {
                let mut runtime = SpawnedRuntime {
                    child,
                    stop_timeout: self.stop_timeout,
                };
                let _ = runtime.try_shutdown().await;
                return Err(error);
            }
        };

        Ok(RuntimeLaunch {
            runtime_id: descriptor.runtime_id.clone(),
            provider_instance_id: descriptor.provider_instance_id.clone(),
            acp_url: descriptor.acp_url.clone(),
            state_stream_url: descriptor.state_stream_url.clone(),
            helper_api_base_url: descriptor.helper_api_base_url.clone(),
            runtime: Box::new(SpawnedRuntime {
                child,
                stop_timeout: self.stop_timeout,
            }),
        })
    }
}

struct SpawnedRuntime {
    child: Child,
    stop_timeout: Duration,
}

impl SpawnedRuntime {
    async fn try_shutdown(&mut self) -> Result<()> {
        if self.child.try_wait()?.is_some() {
            return Ok(());
        }

        send_interrupt(self.child.id()).context("send fireline runtime interrupt")?;

        match tokio::time::timeout(self.stop_timeout, self.child.wait()).await {
            Ok(wait_result) => {
                let _ = wait_result?;
                Ok(())
            }
            Err(_) => {
                self.child
                    .start_kill()
                    .context("force kill fireline runtime")?;
                let _ = self.child.wait().await?;
                Ok(())
            }
        }
    }
}

#[async_trait]
impl ManagedRuntime for SpawnedRuntime {
    async fn shutdown(mut self: Box<Self>) -> Result<()> {
        self.try_shutdown().await
    }
}

#[cfg(unix)]
fn send_interrupt(pid: Option<u32>) -> Result<()> {
    let pid = pid.ok_or_else(|| anyhow!("spawned runtime pid missing"))?;
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid as i32),
        nix::sys::signal::Signal::SIGINT,
    )
    .context("send SIGINT")?;
    Ok(())
}

#[cfg(not(unix))]
fn send_interrupt(_pid: Option<u32>) -> Result<()> {
    Ok(())
}
