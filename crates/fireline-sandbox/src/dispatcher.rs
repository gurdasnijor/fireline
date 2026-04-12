use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use tokio::sync::Mutex;
use tracing::instrument;
use uuid::Uuid;

use crate::provider::{
    Endpoint, HostDescriptor, HostStatus, ManagedSandbox, PersistedHostSpec, ProvisionSpec,
    SandboxLaunch, SandboxProvider, SandboxProviderKind, SandboxProviderRequest,
};
use fireline_session::{HostIndex, StateMaterializer};

#[derive(Clone)]
pub struct SandboxDispatcher {
    providers: Arc<HashMap<SandboxProviderKind, Arc<dyn SandboxProvider>>>,
    read_model: Arc<HostIndex>,
    active_launches: Arc<Mutex<HashMap<String, Box<dyn ManagedSandbox>>>>,
    subscribed_stream_url: Arc<Mutex<Option<String>>>,
}

impl SandboxDispatcher {
    pub fn new(read_model: Arc<HostIndex>, local: Arc<dyn SandboxProvider>) -> Self {
        let mut providers = HashMap::new();
        providers.insert(SandboxProviderKind::Local, local);
        Self {
            providers: Arc::new(providers),
            read_model,
            active_launches: Arc::new(Mutex::new(HashMap::new())),
            subscribed_stream_url: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_provider(mut self, provider: Arc<dyn SandboxProvider>) -> Self {
        Arc::make_mut(&mut self.providers).insert(provider.kind(), provider);
        self
    }

    pub async fn list(&self) -> Vec<HostDescriptor> {
        self.read_model.list_endpoints().await
    }

    pub async fn get(&self, host_key: &str) -> Option<HostDescriptor> {
        self.read_model.endpoints_for(host_key).await
    }

    #[instrument(skip(self, spec), fields(host_key = spec.host_key.as_deref().unwrap_or("<generated>"), provider = ?spec.provider))]
    pub async fn provision(&self, mut spec: ProvisionSpec) -> Result<HostDescriptor> {
        let host_key = spec
            .host_key
            .clone()
            .unwrap_or_else(|| format!("runtime:{}", Uuid::new_v4()));
        let node_id = spec
            .node_id
            .clone()
            .unwrap_or_else(|| node_id_for(spec.host));
        let state_stream_name = spec
            .state_stream
            .clone()
            .unwrap_or_else(|| default_state_stream_name(&host_key));
        spec.host_key = Some(host_key.clone());
        spec.node_id = Some(node_id.clone());
        spec.state_stream = Some(state_stream_name);
        let state_stream_url = state_stream_url(&spec);
        self.ensure_read_model_stream(&state_stream_url).await?;

        let provider = self.resolve(spec.provider)?;
        let provider_kind = provider.kind();
        let launch = provider
            .provision(spec.clone(), host_key.clone(), node_id.clone())
            .await?;
        let descriptor =
            descriptor_from_launch(&host_key, &node_id, provider_kind, &state_stream_url, &launch);
        let persisted_spec = PersistedHostSpec::new(host_key.clone(), node_id, spec);

        crate::stream_trace::emit_host_spec_persisted(&state_stream_url, &persisted_spec).await?;
        crate::stream_trace::emit_host_endpoints_persisted(&state_stream_url, &descriptor).await?;

        let SandboxLaunch { sandbox, .. } = launch;
        self.active_launches.lock().await.insert(host_key, sandbox);

        Ok(descriptor)
    }

    #[instrument(skip(self), fields(host_key))]
    pub async fn stop(&self, host_key: &str) -> Result<HostDescriptor> {
        let descriptor = self
            .get(host_key)
            .await
            .ok_or_else(|| anyhow!("runtime '{host_key}' not found"))?;
        let sandbox = self
            .active_launches
            .lock()
            .await
            .remove(host_key)
            .ok_or_else(|| anyhow!("runtime '{host_key}' is not running"))?;
        sandbox.shutdown().await?;

        let mut stopped = descriptor;
        stopped.status = HostStatus::Stopped;
        stopped.updated_at_ms = now_ms();
        if !stopped.state.url.is_empty() {
            crate::stream_trace::emit_host_endpoints_persisted(&stopped.state.url, &stopped)
                .await?;
        }
        Ok(stopped)
    }

    fn resolve(&self, request: SandboxProviderRequest) -> Result<Arc<dyn SandboxProvider>> {
        match request {
            SandboxProviderRequest::Auto | SandboxProviderRequest::Local => self
                .providers
                .get(&SandboxProviderKind::Local)
                .cloned()
                .ok_or_else(|| anyhow!("local sandbox provider is not configured")),
            SandboxProviderRequest::Docker => self
                .providers
                .get(&SandboxProviderKind::Docker)
                .cloned()
                .ok_or_else(|| anyhow!("docker sandbox provider is not configured")),
        }
    }

    async fn ensure_read_model_stream(&self, state_stream_url: &str) -> Result<()> {
        let mut guard = self.subscribed_stream_url.lock().await;
        match guard.as_deref() {
            Some(existing) if existing == state_stream_url => return Ok(()),
            Some(existing) => {
                return Err(anyhow!(
                    "sandbox dispatcher is already bound to state stream '{existing}'"
                ));
            }
            None => {}
        }

        let materializer = StateMaterializer::new(vec![self.read_model.clone()]);
        let task = materializer.connect(state_stream_url.to_string());
        tokio::time::timeout(Duration::from_secs(5), task.preload())
            .await
            .map_err(|_| anyhow!("timed out preloading sandbox dispatcher read model"))??;
        *guard = Some(state_stream_url.to_string());
        Ok(())
    }
}

fn descriptor_from_launch(
    host_key: &str,
    node_id: &str,
    provider: SandboxProviderKind,
    fallback_state_stream_url: &str,
    launch: &SandboxLaunch,
) -> HostDescriptor {
    let created_at_ms = now_ms();
    let state = if launch.state.url.is_empty() {
        Endpoint::new(fallback_state_stream_url)
    } else {
        launch.state.clone()
    };
    HostDescriptor {
        host_key: host_key.to_string(),
        host_id: launch.host_id.clone(),
        node_id: node_id.to_string(),
        provider,
        provider_instance_id: launch.provider_instance_id.clone(),
        status: if launch.host_id.is_empty() {
            HostStatus::Starting
        } else {
            HostStatus::Ready
        },
        acp: launch.acp.clone(),
        state,
        helper_api_base_url: launch.helper_api_base_url.clone(),
        created_at_ms,
        updated_at_ms: created_at_ms,
    }
}

fn default_state_stream_name(host_key: &str) -> String {
    let key = host_key
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            _ => '-',
        })
        .collect::<String>();
    format!("fireline-state-{key}")
}

fn state_stream_url(spec: &ProvisionSpec) -> String {
    let name = spec
        .state_stream
        .as_deref()
        .expect("state stream name must be normalized before provisioning");
    format!("{}/{}", spec.durable_streams_url.trim_end_matches('/'), name)
}

fn node_id_for(host: std::net::IpAddr) -> String {
    if host.is_unspecified() {
        "node:local".to_string()
    } else {
        format!("node:{host}")
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}
