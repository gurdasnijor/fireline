#![forbid(unsafe_code)]

pub mod manager;
pub mod primitive;
pub mod provider;
pub mod provider_trait;
pub mod providers;
pub mod registry;
pub mod satisfiers;
pub mod stream_trace;

#[cfg(feature = "microsandbox-provider")]
pub mod microsandbox;

pub use fireline_resources::{
    LocalPathMounter, MountedResource, ResourceMounter, ResourceRef, prepare_resources,
};
pub use fireline_session::{StreamStorageConfig, StreamStorageMode};
pub use manager::RuntimeManager;
#[cfg(feature = "microsandbox-provider")]
pub use microsandbox::{MICROSANDBOX_SANDBOX_KIND, MicrosandboxSandbox, MicrosandboxSandboxConfig};
pub use primitive::{Sandbox, SandboxHandle, ToolCall, ToolCallResult};
pub use provider::{
    ProvisionSpec, Endpoint, HeartbeatMetrics, HeartbeatReport, ManagedRuntime,
    PersistedHostSpec, HostDescriptor, RuntimeLaunch, RuntimeProvider, SandboxProviderKind,
    SandboxProviderRequest, HostRegistration, HostStatus, RuntimeTokenIssuer,
};
pub use provider_trait::LocalRuntimeLauncher;
pub use providers::{DockerProvider, DockerProviderConfig, LocalProvider};
pub use registry::RuntimeRegistry;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use tokio::sync::Mutex;
use tracing::{info, instrument};
use uuid::Uuid;

#[derive(Clone)]
pub struct RuntimeHost {
    inner: Arc<RuntimeHostInner>,
}

struct RuntimeHostInner {
    registry: RuntimeRegistry,
    manager: RuntimeManager,
    live_handles: Mutex<HashMap<String, RuntimeLaunch>>,
    pending_host_specs: Mutex<HashMap<String, PersistedHostSpec>>,
}

impl RuntimeHost {
    pub fn new(registry: RuntimeRegistry, manager: RuntimeManager) -> Self {
        Self {
            inner: Arc::new(RuntimeHostInner {
                registry,
                manager,
                live_handles: Mutex::new(HashMap::new()),
                pending_host_specs: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn with_default_registry(manager: RuntimeManager) -> Result<Self> {
        Ok(Self::new(
            RuntimeRegistry::load(RuntimeRegistry::default_path()?)?,
            manager,
        ))
    }

    #[instrument(skip(self, spec), fields(host_key = spec.host_key.as_deref().unwrap_or("<generated>"), provider = ?spec.provider))]
    pub async fn provision(&self, spec: ProvisionSpec) -> Result<HostDescriptor> {
        let host_key = spec
            .host_key
            .clone()
            .unwrap_or_else(|| format!("runtime:{}", Uuid::new_v4()));
        let created_at_ms = now_ms();
        let node_id = spec
            .node_id
            .clone()
            .unwrap_or_else(|| node_id_for(spec.host));
        let provider = self.inner.manager.resolve_kind(spec.provider)?;
        let persisted_spec =
            PersistedHostSpec::new(host_key.clone(), node_id.clone(), spec.clone());

        self.inner.registry.upsert(HostDescriptor {
            host_key: host_key.clone(),
            host_id: String::new(),
            node_id: node_id.clone(),
            provider,
            provider_instance_id: host_key.clone(),
            status: HostStatus::Starting,
            acp: Endpoint::new(""),
            state: Endpoint::new(""),
            helper_api_base_url: None,
            created_at_ms,
            updated_at_ms: created_at_ms,
        })?;

        let (provider, launch) = match self
            .inner
            .manager
            .start(spec.clone(), host_key.clone(), node_id.clone())
            .await
        {
            Ok(started) => started,
            Err(error) => {
                self.inner
                    .pending_host_specs
                    .lock()
                    .await
                    .remove(&host_key);
                let _ = self.inner.registry.remove(&host_key);
                return Err(error);
            }
        };
        self.inner
            .pending_host_specs
            .lock()
            .await
            .insert(host_key.clone(), persisted_spec.clone());
        let launch_host_id = launch.host_id.clone();
        let launch_provider_instance_id = launch.provider_instance_id.clone();
        let launch_acp = launch.acp.clone();
        let launch_state = launch.state.clone();
        let launch_helper_api_base_url = launch.helper_api_base_url.clone();

        self.inner
            .live_handles
            .lock()
            .await
            .insert(host_key.clone(), launch);

        if let Some(descriptor) = self.inner.registry.get(&host_key)?
            && (descriptor.status != HostStatus::Starting || !descriptor.host_id.is_empty())
        {
            let pending_spec = self
                .inner
                .pending_host_specs
                .lock()
                .await
                .get(&host_key)
                .cloned();
            if let Some(spec) = pending_spec
                && !descriptor.state.url.is_empty()
            {
                crate::stream_trace::emit_host_spec_persisted(&descriptor.state.url, &spec)
                    .await?;
                crate::stream_trace::emit_host_endpoints_persisted(
                    &descriptor.state.url,
                    &descriptor,
                )
                .await?;
                self.inner
                    .pending_host_specs
                    .lock()
                    .await
                    .remove(&host_key);
            }
            info!(host_key, status = ?descriptor.status, "runtime host provision returned existing descriptor");
            return Ok(descriptor);
        }

        let descriptor = HostDescriptor {
            host_key: host_key.clone(),
            host_id: launch_host_id,
            node_id,
            provider,
            provider_instance_id: launch_provider_instance_id,
            status: HostStatus::Starting,
            acp: launch_acp,
            state: launch_state,
            helper_api_base_url: launch_helper_api_base_url,
            created_at_ms,
            updated_at_ms: now_ms(),
        };
        self.inner.registry.upsert(descriptor.clone())?;
        if !descriptor.state.url.is_empty() {
            crate::stream_trace::emit_host_spec_persisted(
                &descriptor.state.url,
                &persisted_spec,
            )
            .await?;
            crate::stream_trace::emit_host_endpoints_persisted(
                &descriptor.state.url,
                &descriptor,
            )
            .await?;
            self.inner
                .pending_host_specs
                .lock()
                .await
                .remove(&host_key);
        }

        info!(host_key, status = ?descriptor.status, "runtime host provisioned runtime descriptor");
        Ok(descriptor)
    }

    pub fn get(&self, host_key: &str) -> Result<Option<HostDescriptor>> {
        self.inner.registry.get(host_key)
    }

    pub fn list(&self) -> Result<Vec<HostDescriptor>> {
        self.inner.registry.list()
    }

    #[instrument(skip(self), fields(host_key))]
    pub async fn stop(&self, host_key: &str) -> Result<HostDescriptor> {
        let launch = self
            .inner
            .live_handles
            .lock()
            .await
            .remove(host_key)
            .ok_or_else(|| anyhow!("runtime '{host_key}' is not running"))?;

        launch.runtime.shutdown().await?;

        let mut descriptor = self
            .inner
            .registry
            .get(host_key)?
            .ok_or_else(|| anyhow!("runtime '{host_key}' not found"))?;
        descriptor.status = HostStatus::Stopped;
        descriptor.updated_at_ms = now_ms();
        self.inner.registry.upsert(descriptor.clone())?;
        if !descriptor.state.url.is_empty() {
            crate::stream_trace::emit_host_endpoints_persisted(
                &descriptor.state.url,
                &descriptor,
            )
            .await?;
        }
        info!(host_key, "runtime host stopped runtime");
        Ok(descriptor)
    }

    pub async fn delete(&self, host_key: &str) -> Result<Option<HostDescriptor>> {
        if self
            .inner
            .live_handles
            .lock()
            .await
            .contains_key(host_key)
        {
            self.stop(host_key).await?;
        }

        self.inner.registry.remove(host_key)
    }

    #[instrument(skip(self, registration), fields(host_key))]
    pub async fn register(
        &self,
        host_key: &str,
        registration: HostRegistration,
    ) -> Result<HostDescriptor> {
        let mut descriptor = self
            .inner
            .registry
            .get(host_key)?
            .ok_or_else(|| anyhow!("runtime '{host_key}' not found"))?;

        if descriptor.status == HostStatus::Stopped {
            return Err(anyhow!(
                "runtime '{host_key}' is stopped and cannot re-register"
            ));
        }

        let next_status = match descriptor.status {
            HostStatus::Starting | HostStatus::Stale => HostStatus::Ready,
            HostStatus::Ready => HostStatus::Ready,
            HostStatus::Busy => HostStatus::Busy,
            HostStatus::Idle => HostStatus::Idle,
            HostStatus::Broken => HostStatus::Broken,
            HostStatus::Stopped => unreachable!("stopped runtimes already returned above"),
        };

        descriptor.host_id = registration.host_id;
        descriptor.node_id = registration.node_id;
        descriptor.provider = registration.provider;
        descriptor.provider_instance_id = registration.provider_instance_id;
        descriptor.status = next_status;
        descriptor.acp = Endpoint::new(registration.advertised_acp_url);
        descriptor.state = Endpoint::new(registration.advertised_state_stream_url);
        descriptor.helper_api_base_url = registration.helper_api_base_url;
        descriptor.updated_at_ms = now_ms();

        let pending_spec = self
            .inner
            .pending_host_specs
            .lock()
            .await
            .get(host_key)
            .cloned();
        if let Some(spec) = pending_spec
            && !descriptor.state.url.is_empty()
        {
            crate::stream_trace::emit_host_spec_persisted(&descriptor.state.url, &spec).await?;
            self.inner
                .pending_host_specs
                .lock()
                .await
                .remove(host_key);
        }
        self.inner.registry.upsert(descriptor.clone())?;
        if !descriptor.state.url.is_empty() {
            crate::stream_trace::emit_host_endpoints_persisted(
                &descriptor.state.url,
                &descriptor,
            )
            .await?;
        }
        info!(
            host_key,
            host_id = descriptor.host_id,
            status = ?descriptor.status,
            "runtime registered with host"
        );
        Ok(descriptor)
    }

    pub fn heartbeat(
        &self,
        host_key: &str,
        report: HeartbeatReport,
    ) -> Result<HostDescriptor> {
        let mut descriptor = self
            .inner
            .registry
            .get(host_key)?
            .ok_or_else(|| anyhow!("runtime '{host_key}' not found"))?;

        if descriptor.status == HostStatus::Stopped {
            return Err(anyhow!(
                "runtime '{host_key}' is stopped and cannot heartbeat"
            ));
        }

        if descriptor.status == HostStatus::Stale {
            descriptor.status = HostStatus::Ready;
        }
        descriptor.updated_at_ms = report.ts_ms;
        self.inner.registry.upsert(descriptor.clone())?;
        Ok(descriptor)
    }
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
