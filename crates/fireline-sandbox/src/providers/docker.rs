use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use bollard::Docker;
use bollard::body_full;
use bollard::models::{ContainerCreateBody, HostConfig, PortBinding};
use bollard::query_parameters::{
    BuildImageOptionsBuilder, CreateContainerOptionsBuilder, LogsOptionsBuilder,
    RemoveContainerOptionsBuilder, StartContainerOptions, StopContainerOptionsBuilder,
};
use futures::StreamExt;
use tar::Builder;
use tokio::sync::Mutex;
use url::Url;
use uuid::Uuid;
use walkdir::WalkDir;

use fireline_resources::{LocalPathMounter, ResourceMounter, prepare_resources};

use crate::provider::ManagedSandbox;
use crate::provider_model::{
    ExecutionResult, ProviderCapabilities, SandboxConfig, SandboxDescriptor, SandboxHandle,
    SandboxProvider,
};

const CONTAINER_PORT: u16 = 4437;
const DOCKER_PROVIDER_NAME: &str = "docker";
const LABEL_SANDBOX_ID: &str = "fireline.sandbox_id";
const LABEL_PROVIDER: &str = "fireline.provider";
const READY_LINE_PREFIX: &str = "FIRELINE_READY\t";

#[derive(Debug, Clone)]
pub struct DockerProviderConfig {
    pub image: String,
    pub build_context: PathBuf,
    pub dockerfile: PathBuf,
    pub startup_timeout: Duration,
}

pub struct DockerProvider {
    docker: Docker,
    config: DockerProviderConfig,
    mounters: Vec<Arc<dyn ResourceMounter>>,
    image_ready: Mutex<bool>,
    sandboxes: Arc<Mutex<HashMap<String, DockerSandboxRecord>>>,
}

impl DockerProvider {
    pub fn new(config: DockerProviderConfig) -> Result<Self> {
        Ok(Self {
            docker: Docker::connect_with_local_defaults().context("connect to local docker")?,
            config,
            mounters: vec![Arc::new(LocalPathMounter::new())],
            image_ready: Mutex::new(false),
            sandboxes: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    async fn ensure_image_ready(&self) -> Result<()> {
        let mut guard = self.image_ready.lock().await;
        if *guard {
            return Ok(());
        }

        if self.docker.inspect_image(&self.config.image).await.is_ok() {
            *guard = true;
            return Ok(());
        }

        let build_context = tar_build_context(&self.config.build_context).with_context(|| {
            format!(
                "build docker context tar from {}",
                self.config.build_context.display()
            )
        })?;
        let options = BuildImageOptionsBuilder::default()
            .dockerfile(&self.config.dockerfile.display().to_string())
            .t(&self.config.image)
            .rm(true)
            .pull("true")
            .build();

        let mut stream =
            self.docker
                .build_image(options, None, Some(body_full(build_context.into())));
        while let Some(event) = stream.next().await {
            let event = event.context("build fireline runtime image")?;
            if let Some(error) = event.error {
                return Err(anyhow!("docker build failed: {error}"));
            }
        }

        *guard = true;
        Ok(())
    }

    fn state_stream_name(config: &SandboxConfig, sandbox_id: &str) -> String {
        config
            .state_stream
            .clone()
            .unwrap_or_else(|| format!("fireline-state-{}", sanitize_name(sandbox_id)))
    }

    async fn wait_for_ready_descriptor(
        &self,
        container_name: &str,
        sandbox_id: &str,
    ) -> Result<SandboxDescriptor> {
        let mut logs = self.docker.logs(
            container_name,
            Some(
                LogsOptionsBuilder::default()
                    .follow(true)
                    .stdout(true)
                    .stderr(false)
                    .build(),
            ),
        );
        let deadline = tokio::time::Instant::now() + self.config.startup_timeout;
        let mut buffer = String::new();

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(anyhow!(
                    "timed out waiting for sandbox '{sandbox_id}' to report readiness"
                ));
            }

            match tokio::time::timeout(remaining, logs.next()).await {
                Ok(Some(Ok(output))) => {
                    buffer.push_str(&String::from_utf8_lossy(output.as_ref()));
                    while let Some(newline) = buffer.find('\n') {
                        let line = buffer.drain(..=newline).collect::<String>();
                        let line = line.trim_end();
                        if let Some(payload) = line.strip_prefix(READY_LINE_PREFIX) {
                            return serde_json::from_str(payload)
                                .context("decode docker sandbox readiness descriptor");
                        }
                        if !line.is_empty() {
                            tracing::info!(
                                sandbox_id,
                                line,
                                "docker sandbox stdout before readiness"
                            );
                        }
                    }
                }
                Ok(Some(Err(error))) => {
                    return Err(anyhow::Error::from(error))
                        .context("read docker sandbox startup logs");
                }
                Ok(None) => {
                    return Err(anyhow!(
                        "docker sandbox exited before reporting readiness for sandbox '{sandbox_id}'"
                    ));
                }
                Err(_) => {
                    return Err(anyhow!(
                        "timed out waiting for sandbox '{sandbox_id}' to report readiness"
                    ));
                }
            }
        }
    }
}

#[async_trait]
impl SandboxProvider for DockerProvider {
    fn name(&self) -> &str {
        DOCKER_PROVIDER_NAME
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            file_transfer: true,
            stream_resources: true,
            oci_images: true,
            ..ProviderCapabilities::default()
        }
    }

    async fn create(&self, config: &SandboxConfig) -> Result<SandboxHandle> {
        self.ensure_image_ready().await?;
        let sandbox_id = format!("runtime:{}", Uuid::new_v4());
        let mounted_resources =
            prepare_resources(&config.resources, &self.mounters, &sandbox_id).await?;

        let container_name = format!("fireline-{}", sanitize_name(&sandbox_id));
        let published_port = reserve_host_port()?;
        let state_stream_name = Self::state_stream_name(config, &sandbox_id);
        let advertised_acp_url = format!("ws://127.0.0.1:{published_port}/acp");
        let advertised_state_stream_url =
            join_stream_url(&config.durable_streams_url, &state_stream_name);
        let connect_durable_streams_url =
            rewrite_loopback_for_container(&config.durable_streams_url)?;
        let connect_control_plane_url = config
            .control_plane_url
            .as_deref()
            .map(rewrite_loopback_for_container)
            .transpose()?;

        let agent_command = rewrite_agent_command_for_image(&config.agent_command)?;
        let bind_mounts = mounted_resources
            .iter()
            .map(|resource| {
                format!(
                    "{}:{}:{}",
                    resource.host_path.display(),
                    resource.mount_path.display(),
                    if resource.read_only { "ro" } else { "rw" }
                )
            })
            .collect::<Vec<_>>();
        let mut cmd = vec![
            "--host".to_string(),
            "0.0.0.0".to_string(),
            "--port".to_string(),
            CONTAINER_PORT.to_string(),
            "--name".to_string(),
            config.name.clone(),
            "--durable-streams-url".to_string(),
            connect_durable_streams_url,
            "--state-stream".to_string(),
            state_stream_name,
            "--".to_string(),
        ];
        cmd.extend(agent_command);

        if !config.topology.components.is_empty() {
            let topology_json =
                serde_json::to_string(&config.topology).context("serialize runtime topology")?;
            cmd.splice(
                cmd.len() - 1..cmd.len() - 1,
                ["--topology-json".to_string(), topology_json],
            );
        }

        if !mounted_resources.is_empty() {
            let mounted_resources_json =
                serde_json::to_string(&mounted_resources).context("serialize mounted resources")?;
            cmd.splice(
                cmd.len() - 1..cmd.len() - 1,
                [
                    "--mounted-resources-json".to_string(),
                    mounted_resources_json,
                ],
            );
        }

        let mut env_vars = vec![
            format!("FIRELINE_RUNTIME_KEY={sandbox_id}"),
            "FIRELINE_NODE_ID=node:docker".to_string(),
            "FIRELINE_PROVIDER_KIND=docker".to_string(),
            format!("FIRELINE_ADVERTISED_ACP_URL={advertised_acp_url}"),
            format!("FIRELINE_ADVERTISED_STATE_STREAM_URL={advertised_state_stream_url}"),
            "FIRELINE_TRANSLATE_SESSION_CWD_TO_MOUNTS=1".to_string(),
        ];
        if let Some(control_plane_url) = connect_control_plane_url {
            env_vars.push(format!("FIRELINE_CONTROL_PLANE_URL={control_plane_url}"));
        }
        env_vars.extend(
            config
                .env_vars
                .iter()
                .map(|(key, value)| format!("{key}={value}")),
        );

        let config = ContainerCreateBody {
            image: Some(self.config.image.clone()),
            cmd: Some(cmd),
            env: Some(env_vars),
            exposed_ports: Some(HashMap::from([(
                format!("{CONTAINER_PORT}/tcp"),
                HashMap::new(),
            )])),
            labels: Some(HashMap::from([
                (LABEL_SANDBOX_ID.to_string(), sandbox_id.clone()),
                (LABEL_PROVIDER.to_string(), DOCKER_PROVIDER_NAME.to_string()),
            ])),
            host_config: Some(HostConfig {
                binds: (!bind_mounts.is_empty()).then_some(bind_mounts),
                port_bindings: Some(HashMap::from([(
                    format!("{CONTAINER_PORT}/tcp"),
                    Some(vec![PortBinding {
                        host_ip: Some("127.0.0.1".to_string()),
                        host_port: Some(published_port.to_string()),
                    }]),
                )])),
                extra_hosts: Some(vec!["host.docker.internal:host-gateway".to_string()]),
                auto_remove: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };

        let create_options = CreateContainerOptionsBuilder::default()
            .name(&container_name)
            .build();
        self.docker
            .create_container(Some(create_options), config)
            .await
            .with_context(|| format!("create docker runtime container {container_name}"))?;

        self.docker
            .start_container(&container_name, None::<StartContainerOptions>)
            .await
            .with_context(|| format!("start docker runtime container {container_name}"))?;

        let descriptor = match self
            .wait_for_ready_descriptor(&container_name, &sandbox_id)
            .await
        {
            Ok(descriptor) => descriptor,
            Err(error) => {
                let sandbox = Box::new(DockerManagedSandbox {
                    docker: self.docker.clone(),
                    container_name,
                });
                if let Err(shutdown_error) = sandbox.shutdown().await {
                    return Err(error.context(format!(
                        "cleanup docker sandbox after startup failure also failed: {shutdown_error:#}"
                    )));
                }
                return Err(error);
            }
        };

        let managed_sandbox = Box::new(DockerManagedSandbox {
            docker: self.docker.clone(),
            container_name,
        });
        let handle = SandboxHandle::from_descriptor(descriptor.clone(), self.name());
        self.sandboxes.lock().await.insert(
            sandbox_id,
            DockerSandboxRecord {
                descriptor,
                sandbox: managed_sandbox,
            },
        );
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
            "docker sandbox '{id}' does not yet support provider-model execute()"
        ))
    }

    async fn destroy(&self, id: &str) -> Result<bool> {
        let Some(record) = self.sandboxes.lock().await.remove(id) else {
            return Ok(false);
        };
        record.sandbox.shutdown().await?;
        Ok(true)
    }

    async fn health_check(&self) -> Result<bool> {
        Ok(self.docker.version().await.is_ok())
    }
}

struct DockerSandboxRecord {
    descriptor: SandboxDescriptor,
    sandbox: Box<dyn ManagedSandbox>,
}

struct DockerManagedSandbox {
    docker: Docker,
    container_name: String,
}

#[async_trait]
impl ManagedSandbox for DockerManagedSandbox {
    async fn shutdown(self: Box<Self>) -> Result<()> {
        let _ = self
            .docker
            .stop_container(
                &self.container_name,
                Some(StopContainerOptionsBuilder::default().t(10).build()),
            )
            .await;
        let _ = self
            .docker
            .remove_container(
                &self.container_name,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await;
        Ok(())
    }
}

fn reserve_host_port() -> Result<u16> {
    let listener =
        TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).context("bind")?;
    Ok(listener.local_addr()?.port())
}

fn sanitize_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => ch,
            _ => '-',
        })
        .collect()
}

fn rewrite_loopback_for_container(url: &str) -> Result<String> {
    let mut url = Url::parse(url).with_context(|| format!("parse url '{url}'"))?;
    if matches!(
        url.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("0.0.0.0") | Some("[::1]") | Some("::1")
    ) {
        url.set_host(Some("host.docker.internal"))
            .map_err(|_| anyhow!("rewrite host for docker container"))?;
    }
    Ok(url.to_string())
}

fn join_stream_url(base: &str, stream_name: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), stream_name)
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

fn rewrite_agent_command_for_image(agent_command: &[String]) -> Result<Vec<String>> {
    let Some((first, rest)) = agent_command.split_first() else {
        return Err(anyhow!("docker runtime requires a non-empty agent command"));
    };

    let rewritten = if Path::new(first).is_absolute() {
        let file_name = Path::new(first)
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| anyhow!("docker runtime agent command must have a valid executable"))?;
        match file_name {
            "fireline-testy" | "fireline-testy-load" | "fireline-testy-prompt" => {
                format!("/usr/local/bin/{file_name}")
            }
            _ => {
                return Err(anyhow!(
                    "docker runtime cannot execute host-only binary '{}'",
                    first
                ));
            }
        }
    } else {
        first.clone()
    };

    Ok(std::iter::once(rewritten)
        .chain(rest.iter().cloned())
        .collect())
}

fn tar_build_context(root: &Path) -> Result<Vec<u8>> {
    let mut archive = Builder::new(Vec::new());
    for entry in WalkDir::new(root).into_iter().filter_entry(|entry| {
        let name = entry.file_name().to_string_lossy();
        !matches!(name.as_ref(), ".git" | "target" | "node_modules")
    }) {
        let entry = entry?;
        let path = entry.path();
        if path == root {
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("strip build context root {}", root.display()))?;

        if entry.file_type().is_dir() {
            archive
                .append_dir(relative, path)
                .with_context(|| format!("append docker build dir {}", path.display()))?;
            continue;
        }

        if entry.file_type().is_file() {
            let mut file = File::open(path)
                .with_context(|| format!("open build context {}", path.display()))?;
            let mut contents = Vec::new();
            file.read_to_end(&mut contents)
                .with_context(|| format!("read build context {}", path.display()))?;
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append_data(&mut header, relative, contents.as_slice())
                .with_context(|| format!("append docker build file {}", path.display()))?;
        }
    }

    archive
        .into_inner()
        .context("finalize docker build context archive")
}
