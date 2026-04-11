use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use bollard::body_full;
use bollard::models::{ContainerCreateBody, HostConfig, PortBinding};
use bollard::query_parameters::{
    BuildImageOptionsBuilder, CreateContainerOptionsBuilder, RemoveContainerOptionsBuilder,
    StartContainerOptions, StopContainerOptionsBuilder,
};
use bollard::Docker;
use futures::StreamExt;
use tar::Builder;
use tokio::sync::Mutex;
use url::Url;
use walkdir::WalkDir;

use super::provider::{
    CreateRuntimeSpec, Endpoint, ManagedRuntime, RuntimeLaunch, RuntimeProvider, RuntimeProviderKind,
    RuntimeTokenIssuer,
};

const CONTAINER_PORT: u16 = 4437;
const TOKEN_TTL: Duration = Duration::from_secs(60 * 60 * 24);
const LABEL_RUNTIME_KEY: &str = "fireline.runtime_key";
const LABEL_PROVIDER: &str = "fireline.provider";

#[derive(Debug, Clone)]
pub struct DockerProviderConfig {
    pub control_plane_url: String,
    pub shared_stream_base_url: Option<String>,
    pub image: String,
    pub build_context: PathBuf,
    pub dockerfile: PathBuf,
}

pub struct DockerProvider {
    docker: Docker,
    config: DockerProviderConfig,
    token_issuer: Arc<dyn RuntimeTokenIssuer>,
    image_ready: Mutex<bool>,
}

impl DockerProvider {
    pub fn new(
        config: DockerProviderConfig,
        token_issuer: Arc<dyn RuntimeTokenIssuer>,
    ) -> Result<Self> {
        Ok(Self {
            docker: Docker::connect_with_local_defaults().context("connect to local docker")?,
            config,
            token_issuer,
            image_ready: Mutex::new(false),
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

    fn state_stream_name(spec: &CreateRuntimeSpec, runtime_key: &str) -> String {
        spec.state_stream
            .clone()
            .unwrap_or_else(|| format!("fireline-state-{}", sanitize_name(runtime_key)))
    }
}

#[async_trait]
impl RuntimeProvider for DockerProvider {
    fn kind(&self) -> RuntimeProviderKind {
        RuntimeProviderKind::Docker
    }

    async fn start(
        &self,
        spec: CreateRuntimeSpec,
        runtime_key: String,
        node_id: String,
    ) -> Result<RuntimeLaunch> {
        self.ensure_image_ready().await?;

        let control_plane_url = rewrite_loopback_for_container(&self.config.control_plane_url)?;
        let container_name = format!("fireline-{}", sanitize_name(&runtime_key));
        let provider_instance_id = container_name.clone();
        let published_port = reserve_host_port()?;
        let state_stream_name = Self::state_stream_name(&spec, &runtime_key);
        let advertised_acp_url = format!("ws://127.0.0.1:{published_port}/acp");
        let advertised_state_stream_url = self
            .config
            .shared_stream_base_url
            .as_ref()
            .map(|base| join_stream_url(base, &state_stream_name))
            .unwrap_or_else(|| {
                format!(
                    "http://127.0.0.1:{published_port}/v1/stream/{state_stream_name}"
                )
            });
        let connect_state_stream_url = self
            .config
            .shared_stream_base_url
            .as_ref()
            .map(|base| rewrite_loopback_for_container(&join_stream_url(base, &state_stream_name)))
            .transpose()?;

        let runtime_token = self.token_issuer.issue(&runtime_key, TOKEN_TTL);
        let agent_command = rewrite_agent_command_for_image(&spec.agent_command)?;
        let mut cmd = vec![
            "--host".to_string(),
            "0.0.0.0".to_string(),
            "--port".to_string(),
            CONTAINER_PORT.to_string(),
            "--name".to_string(),
            spec.name.clone(),
            "--state-stream".to_string(),
            state_stream_name,
            "--".to_string(),
        ];
        cmd.extend(agent_command);

        if !spec.topology.components.is_empty() {
            let topology_json =
                serde_json::to_string(&spec.topology).context("serialize runtime topology")?;
            cmd.splice(
                cmd.len() - 1..cmd.len() - 1,
                [
                    "--topology-json".to_string(),
                    topology_json,
                ],
            );
        }

        let config = ContainerCreateBody {
            image: Some(self.config.image.clone()),
            cmd: Some(cmd),
            env: Some(
                [
                    format!("FIRELINE_RUNTIME_KEY={runtime_key}"),
                    format!("FIRELINE_NODE_ID={node_id}"),
                    format!("FIRELINE_CONTROL_PLANE_URL={control_plane_url}"),
                    format!("FIRELINE_CONTROL_PLANE_TOKEN={runtime_token}"),
                    "FIRELINE_PROVIDER_KIND=docker".to_string(),
                    format!("FIRELINE_PROVIDER_INSTANCE_ID={provider_instance_id}"),
                    format!("FIRELINE_ADVERTISED_ACP_URL={advertised_acp_url}"),
                    format!(
                        "FIRELINE_ADVERTISED_STATE_STREAM_URL={advertised_state_stream_url}"
                    ),
                ]
                .into_iter()
                .chain(connect_state_stream_url.into_iter().map(|url| {
                    format!("FIRELINE_EXTERNAL_STATE_STREAM_URL={url}")
                }))
                .collect(),
            ),
            exposed_ports: Some(HashMap::from([(
                format!("{CONTAINER_PORT}/tcp"),
                HashMap::new(),
            )])),
            labels: Some(HashMap::from([
                (LABEL_RUNTIME_KEY.to_string(), runtime_key.clone()),
                (LABEL_PROVIDER.to_string(), "docker".to_string()),
            ])),
            host_config: Some(HostConfig {
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

        Ok(RuntimeLaunch {
            status: super::RuntimeStatus::Starting,
            runtime_id: String::new(),
            provider_instance_id,
            acp: Endpoint::new(advertised_acp_url),
            state: Endpoint::new(advertised_state_stream_url),
            helper_api_base_url: None,
            runtime: Box::new(DockerManagedRuntime {
                docker: self.docker.clone(),
                container_name,
            }),
        })
    }
}

struct DockerManagedRuntime {
    docker: Docker,
    container_name: String,
}

#[async_trait]
impl ManagedRuntime for DockerManagedRuntime {
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
    value.chars()
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
            let mut file =
                File::open(path).with_context(|| format!("open build context {}", path.display()))?;
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
