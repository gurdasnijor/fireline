use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use fireline_resources::{LocalPathMounter, MountedResource, ResourceMounter, prepare_resources};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::info;
use uuid::Uuid;

use crate::provider::ManagedSandbox;
use crate::provider_model::{
    ExecutionResult, ProviderCapabilities, SandboxConfig, SandboxDescriptor, SandboxHandle,
    SandboxProvider, SandboxStatus,
};

const READY_LINE_PREFIX: &str = "FIRELINE_READY\t";
const LOCAL_PROVIDER_NAME: &str = "local";

#[derive(Debug, Clone)]
pub struct LocalSubprocessProviderConfig {
    pub fireline_bin: PathBuf,
    pub host: IpAddr,
    pub default_peer_directory_path: Option<PathBuf>,
    pub startup_timeout: Duration,
    pub stop_timeout: Duration,
}

#[derive(Clone)]
pub struct LocalSubprocessProvider {
    config: LocalSubprocessProviderConfig,
    mounters: Vec<Arc<dyn ResourceMounter>>,
    sandboxes: Arc<Mutex<HashMap<String, LocalSandboxRecord>>>,
}

impl LocalSubprocessProvider {
    pub fn new(config: LocalSubprocessProviderConfig) -> Self {
        Self::with_mounters(config, vec![Arc::new(LocalPathMounter::new())])
    }

    pub fn with_mounters(
        config: LocalSubprocessProviderConfig,
        mounters: Vec<Arc<dyn ResourceMounter>>,
    ) -> Self {
        Self {
            config,
            mounters,
            sandboxes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn wait_for_ready_descriptor(
        &self,
        sandbox_id: &str,
        child: &mut Child,
    ) -> Result<(SandboxDescriptor, JoinHandle<()>)> {
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("spawned sandbox stdout missing"))?;
        let mut lines = BufReader::new(stdout).lines();
        let deadline = tokio::time::Instant::now() + self.config.startup_timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(anyhow!(
                    "timed out waiting for sandbox '{sandbox_id}' to report readiness"
                ));
            }

            match tokio::time::timeout(remaining, lines.next_line()).await {
                Ok(Ok(Some(line))) => {
                    if let Some(payload) = line.strip_prefix(READY_LINE_PREFIX) {
                        let descriptor: SandboxDescriptor = serde_json::from_str(payload)
                            .context("decode child sandbox readiness descriptor")?;
                        info!(sandbox_id, "child-process sandbox became ready");
                        let drain_task = tokio::spawn(async move {
                            while matches!(lines.next_line().await, Ok(Some(_))) {}
                        });
                        return Ok((descriptor, drain_task));
                    }
                    if !line.trim().is_empty() {
                        info!(sandbox_id, line, "child sandbox stdout before readiness");
                    }
                }
                Ok(Ok(None)) => {
                    let status = child.wait().await?;
                    return Err(anyhow!(
                        "fireline sandbox exited before reporting readiness: {status}"
                    ));
                }
                Ok(Err(error)) => {
                    return Err(anyhow::Error::from(error)).context("read child sandbox stdout");
                }
                Err(_) => {
                    return Err(anyhow!(
                        "timed out waiting for sandbox '{sandbox_id}' to report readiness"
                    ));
                }
            }
        }
    }

    async fn launch_local_sandbox(
        &self,
        config: &SandboxConfig,
        sandbox_id: &str,
        mounted_resources: &[MountedResource],
    ) -> Result<LocalSandboxRecord> {
        let node_id = node_id_for(self.config.host);
        let state_stream_name = config
            .state_stream
            .clone()
            .unwrap_or_else(|| format!("fireline-state-{}", sanitize_state_stream_key(sandbox_id)));
        let advertised_state_stream_url =
            join_stream_url(&config.durable_streams_url, &state_stream_name);

        let mut command = Command::new(&self.config.fireline_bin);
        command
            .arg("--host")
            .arg(self.config.host.to_string())
            .arg("--port")
            .arg("0")
            .arg("--name")
            .arg(&config.name)
            .arg("--host-key")
            .arg(sandbox_id)
            .arg("--node-id")
            .arg(&node_id)
            .arg("--durable-streams-url")
            .arg(&config.durable_streams_url)
            .arg("--state-stream")
            .arg(&state_stream_name)
            .env(
                "FIRELINE_ADVERTISED_STATE_STREAM_URL",
                &advertised_state_stream_url,
            );

        if let Some(peer_directory_path) = self.config.default_peer_directory_path.as_ref() {
            command
                .arg("--peer-directory-path")
                .arg(peer_directory_path);
        }

        if let Some(control_plane_url) = config.control_plane_url.as_ref() {
            command.env("FIRELINE_CONTROL_PLANE_URL", control_plane_url);
        }

        if !config.topology.components.is_empty() {
            command
                .arg("--topology-json")
                .arg(serde_json::to_string(&config.topology).context("serialize topology")?);
        }

        if !mounted_resources.is_empty() {
            command.arg("--mounted-resources-json").arg(
                serde_json::to_string(mounted_resources).context("serialize mounted resources")?,
            );
        }

        for (key, value) in &config.env_vars {
            command.env(key, value);
        }

        command
            .arg("--")
            .args(&config.agent_command)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        let mut child = command.spawn().with_context(|| {
            format!(
                "spawn fireline binary {}",
                self.config.fireline_bin.display()
            )
        })?;

        let (descriptor, stdout_drain_task) = match self
            .wait_for_ready_descriptor(sandbox_id, &mut child)
            .await
        {
            Ok(ready) => ready,
            Err(error) => {
                let mut sandbox = SpawnedSandbox {
                    child,
                    stop_timeout: self.config.stop_timeout,
                    stdout_drain_task: None,
                };
                if let Err(shutdown_error) = sandbox.try_shutdown().await {
                    tracing::warn!(
                        sandbox_id,
                        error = %shutdown_error,
                        "cleanup child sandbox after startup failure failed"
                    );
                    return Err(error.context(format!(
                            "cleanup child sandbox after startup failure also failed: {shutdown_error:#}"
                        )));
                }
                return Err(error);
            }
        };

        Ok(LocalSandboxRecord {
            descriptor,
            sandbox: Box::new(SpawnedSandbox {
                child,
                stop_timeout: self.config.stop_timeout,
                stdout_drain_task: Some(stdout_drain_task),
            }),
        })
    }
}

#[async_trait]
impl SandboxProvider for LocalSubprocessProvider {
    fn name(&self) -> &str {
        LOCAL_PROVIDER_NAME
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            file_transfer: true,
            stream_resources: true,
            ..ProviderCapabilities::default()
        }
    }

    async fn create(&self, config: &SandboxConfig) -> Result<SandboxHandle> {
        let sandbox_id = format!("runtime:{}", Uuid::new_v4());
        let mounted_resources = prepare_resources(&config.resources, &self.mounters, &sandbox_id)
            .await
            .with_context(|| format!("prepare resources for sandbox '{sandbox_id}'"))?;
        let record = self
            .launch_local_sandbox(config, &sandbox_id, &mounted_resources)
            .await?;
        let handle = SandboxHandle::from_descriptor(record.descriptor.clone(), self.name());
        self.sandboxes.lock().await.insert(sandbox_id, record);
        Ok(handle)
    }

    async fn get(&self, id: &str) -> Result<Option<SandboxDescriptor>> {
        Ok(self
            .sandboxes
            .lock()
            .await
            .get(id)
            .map(|record| record.descriptor.clone()))
    }

    async fn list(
        &self,
        labels: Option<&HashMap<String, String>>,
    ) -> Result<Vec<SandboxDescriptor>> {
        let mut descriptors: Vec<_> = self
            .sandboxes
            .lock()
            .await
            .values()
            .map(|record| record.descriptor.clone())
            .filter(|descriptor| labels_match(&descriptor.labels, labels))
            .collect();
        descriptors.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(descriptors)
    }

    async fn execute(
        &self,
        id: &str,
        _command: &str,
        _timeout: Option<Duration>,
        _env: Option<&HashMap<String, String>>,
    ) -> Result<ExecutionResult> {
        Err(anyhow!(
            "local subprocess sandbox '{id}' does not yet support provider-model execute()"
        ))
    }

    async fn destroy(&self, id: &str) -> Result<bool> {
        let Some(mut record) = self.sandboxes.lock().await.remove(id) else {
            return Ok(false);
        };

        record.sandbox.shutdown().await?;
        record.descriptor.status = SandboxStatus::Stopped;
        record.descriptor.updated_at_ms = now_ms();
        Ok(true)
    }

    async fn health_check(&self) -> Result<bool> {
        Ok(self.config.fireline_bin.exists())
    }
}

struct LocalSandboxRecord {
    descriptor: SandboxDescriptor,
    sandbox: Box<dyn ManagedSandbox>,
}

struct SpawnedSandbox {
    child: Child,
    stop_timeout: Duration,
    stdout_drain_task: Option<JoinHandle<()>>,
}

impl SpawnedSandbox {
    async fn try_shutdown(&mut self) -> Result<()> {
        if self.child.try_wait()?.is_some() {
            self.finish_stdout_drain().await;
            return Ok(());
        }

        send_interrupt(self.child.id()).context("send fireline sandbox interrupt")?;

        match tokio::time::timeout(self.stop_timeout, self.child.wait()).await {
            Ok(wait_result) => {
                let _ = wait_result?;
            }
            Err(_) => {
                self.child
                    .start_kill()
                    .context("force kill fireline sandbox")?;
                let _ = self.child.wait().await?;
            }
        }
        self.finish_stdout_drain().await;
        Ok(())
    }

    async fn finish_stdout_drain(&mut self) {
        if let Some(task) = self.stdout_drain_task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

#[async_trait]
impl ManagedSandbox for SpawnedSandbox {
    async fn shutdown(mut self: Box<Self>) -> Result<()> {
        self.try_shutdown().await
    }
}

#[cfg(unix)]
fn send_interrupt(pid: Option<u32>) -> Result<()> {
    let pid = pid.ok_or_else(|| anyhow!("spawned sandbox pid missing"))?;
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

fn labels_match(
    actual: &HashMap<String, String>,
    expected: Option<&HashMap<String, String>>,
) -> bool {
    let Some(expected) = expected else {
        return true;
    };

    expected.iter().all(|(key, value)| {
        actual
            .get(key)
            .is_some_and(|actual_value| actual_value == value)
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}

fn node_id_for(host: IpAddr) -> String {
    if host.is_unspecified() {
        "node:local".to_string()
    } else {
        format!("node:{host}")
    }
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

fn join_stream_url(base_url: &str, stream_name: &str) -> String {
    format!("{}/{}", base_url.trim_end_matches('/'), stream_name)
}
