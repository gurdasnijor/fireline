use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use fireline_conductor::runtime::{
    CreateRuntimeSpec, LocalRuntimeLauncher, ManagedRuntime, MountedResource, RuntimeDescriptor,
    RuntimeLaunch, RuntimeRegistry, RuntimeStatus,
};
use tokio::process::{Child, Command};
use tracing::{info, instrument};

use crate::auth::RuntimeTokenStore;

pub struct ChildProcessRuntimeLauncher {
    fireline_bin: PathBuf,
    runtime_registry: RuntimeRegistry,
    runtime_registry_path: PathBuf,
    default_peer_directory_path: Option<PathBuf>,
    prefer_push: bool,
    control_plane_url: String,
    shared_stream_base_url: Option<String>,
    token_store: RuntimeTokenStore,
    startup_timeout: Duration,
    stop_timeout: Duration,
    poll_interval: Duration,
}

impl ChildProcessRuntimeLauncher {
    // The local launcher is assembled from CLI/runtime wiring rather than a config object.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        fireline_bin: PathBuf,
        runtime_registry: RuntimeRegistry,
        runtime_registry_path: PathBuf,
        default_peer_directory_path: Option<PathBuf>,
        prefer_push: bool,
        control_plane_url: String,
        shared_stream_base_url: Option<String>,
        token_store: RuntimeTokenStore,
        startup_timeout: Duration,
        stop_timeout: Duration,
    ) -> Self {
        Self {
            fireline_bin,
            runtime_registry,
            runtime_registry_path,
            default_peer_directory_path,
            prefer_push,
            control_plane_url,
            shared_stream_base_url,
            token_store,
            startup_timeout,
            stop_timeout,
            poll_interval: Duration::from_millis(100),
        }
    }

    #[instrument(skip(self, child), fields(runtime_key))]
    async fn wait_for_runtime_ready(
        &self,
        runtime_key: &str,
        child: &mut Child,
    ) -> Result<RuntimeDescriptor> {
        let deadline = tokio::time::Instant::now() + self.startup_timeout;
        let mut polls = 0usize;
        loop {
            polls += 1;
            if let Some(runtime) = self.runtime_registry.get(runtime_key)? {
                match runtime.status {
                    RuntimeStatus::Ready => {
                        info!(runtime_key, polls, "child-process runtime became ready");
                        return Ok(runtime);
                    }
                    RuntimeStatus::Broken | RuntimeStatus::Stopped => {
                        return Err(anyhow!(
                            "fireline runtime failed during startup with status '{:?}'",
                            runtime.status
                        ));
                    }
                    _ => {}
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
    #[instrument(
        skip(self, spec, mounted_resources),
        fields(runtime_key, node_id, provider = "local")
    )]
    async fn start_local_runtime(
        &self,
        spec: CreateRuntimeSpec,
        runtime_key: String,
        node_id: String,
        mounted_resources: Vec<MountedResource>,
    ) -> Result<RuntimeLaunch> {
        let state_stream_name = spec.state_stream.clone().unwrap_or_else(|| {
            format!("fireline-state-{}", sanitize_state_stream_key(&runtime_key))
        });
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
            .arg(&self.runtime_registry_path)
            .arg("--state-stream")
            .arg(&state_stream_name);

        if self.prefer_push {
            let runtime_token = self
                .token_store
                .issue(&runtime_key, Duration::from_secs(60 * 60 * 24));
            command
                .arg("--control-plane-url")
                .arg(&self.control_plane_url)
                .env("FIRELINE_CONTROL_PLANE_TOKEN", runtime_token.token);
        }

        if let Some(shared_stream_base_url) = &self.shared_stream_base_url {
            let state_stream_url = join_stream_url(shared_stream_base_url, &state_stream_name);
            command
                .env("FIRELINE_EXTERNAL_STATE_STREAM_URL", &state_stream_url)
                .env("FIRELINE_ADVERTISED_STATE_STREAM_URL", state_stream_url);
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

        if !mounted_resources.is_empty() {
            command.arg("--mounted-resources-json").arg(
                serde_json::to_string(&mounted_resources).context("serialize mounted resources")?,
            );
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

        Ok(RuntimeLaunch::ready(
            descriptor.runtime_id.clone(),
            descriptor.provider_instance_id.clone(),
            descriptor.acp.clone(),
            descriptor.state.clone(),
            descriptor.helper_api_base_url.clone(),
            Box::new(SpawnedRuntime {
                child,
                stop_timeout: self.stop_timeout,
            }),
        ))
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

fn sanitize_state_stream_key(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            _ => '-',
        })
        .collect()
}

fn join_stream_url(base: &str, stream_name: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), stream_name)
}
