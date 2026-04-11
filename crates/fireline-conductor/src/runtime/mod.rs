mod docker;
mod local;
mod manager;
mod provider;
mod registry;

pub use self::docker::{DockerProvider, DockerProviderConfig};
pub use self::local::{LocalProvider, LocalRuntimeLauncher};
pub use self::manager::RuntimeManager;
pub use self::provider::{
    CreateRuntimeSpec, Endpoint, HeartbeatMetrics, HeartbeatReport, ManagedRuntime,
    RuntimeDescriptor, RuntimeLaunch, RuntimeProvider, RuntimeProviderKind, RuntimeProviderRequest,
    RuntimeRegistration, RuntimeStatus, RuntimeTokenIssuer, StreamStorageConfig, StreamStorageMode,
};
pub use self::registry::RuntimeRegistry;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone)]
pub struct RuntimeHost {
    inner: Arc<RuntimeHostInner>,
}

struct RuntimeHostInner {
    registry: RuntimeRegistry,
    manager: RuntimeManager,
    live_handles: Mutex<HashMap<String, RuntimeLaunch>>,
}

impl RuntimeHost {
    pub fn new(registry: RuntimeRegistry, manager: RuntimeManager) -> Self {
        Self {
            inner: Arc::new(RuntimeHostInner {
                registry,
                manager,
                live_handles: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn with_default_registry(manager: RuntimeManager) -> Result<Self> {
        Ok(Self::new(
            RuntimeRegistry::load(RuntimeRegistry::default_path()?)?,
            manager,
        ))
    }

    pub async fn create(&self, spec: CreateRuntimeSpec) -> Result<RuntimeDescriptor> {
        let runtime_key = format!("runtime:{}", Uuid::new_v4());
        let created_at_ms = now_ms();
        let node_id = node_id_for(spec.host);
        let provider = self.inner.manager.resolve_kind(spec.provider)?;

        self.inner.registry.upsert(RuntimeDescriptor {
            runtime_key: runtime_key.clone(),
            runtime_id: String::new(),
            node_id: node_id.clone(),
            provider,
            provider_instance_id: runtime_key.clone(),
            status: RuntimeStatus::Starting,
            acp: Endpoint::new(""),
            state: Endpoint::new(""),
            helper_api_base_url: None,
            created_at_ms,
            updated_at_ms: created_at_ms,
        })?;

        let (provider, launch) = match self
            .inner
            .manager
            .start(spec, runtime_key.clone(), node_id.clone())
            .await
        {
            Ok(started) => started,
            Err(error) => {
                let _ = self.inner.registry.remove(&runtime_key);
                return Err(error);
            }
        };
        let launch_runtime_id = launch.runtime_id.clone();
        let launch_provider_instance_id = launch.provider_instance_id.clone();
        let launch_acp = launch.acp.clone();
        let launch_state = launch.state.clone();
        let launch_helper_api_base_url = launch.helper_api_base_url.clone();

        self.inner
            .live_handles
            .lock()
            .await
            .insert(runtime_key.clone(), launch);

        if let Some(descriptor) = self.inner.registry.get(&runtime_key)?
            && (descriptor.status != RuntimeStatus::Starting || !descriptor.runtime_id.is_empty())
        {
            return Ok(descriptor);
        }

        let descriptor = RuntimeDescriptor {
            runtime_key: runtime_key.clone(),
            runtime_id: launch_runtime_id,
            node_id,
            provider,
            provider_instance_id: launch_provider_instance_id,
            status: RuntimeStatus::Starting,
            acp: launch_acp,
            state: launch_state,
            helper_api_base_url: launch_helper_api_base_url,
            created_at_ms,
            updated_at_ms: now_ms(),
        };
        self.inner.registry.upsert(descriptor.clone())?;

        Ok(descriptor)
    }

    pub fn get(&self, runtime_key: &str) -> Result<Option<RuntimeDescriptor>> {
        self.inner.registry.get(runtime_key)
    }

    pub fn list(&self) -> Result<Vec<RuntimeDescriptor>> {
        self.inner.registry.list()
    }

    pub async fn stop(&self, runtime_key: &str) -> Result<RuntimeDescriptor> {
        let launch = self
            .inner
            .live_handles
            .lock()
            .await
            .remove(runtime_key)
            .ok_or_else(|| anyhow!("runtime '{runtime_key}' is not running"))?;

        launch.runtime.shutdown().await?;

        let mut descriptor = self
            .inner
            .registry
            .get(runtime_key)?
            .ok_or_else(|| anyhow!("runtime '{runtime_key}' not found"))?;
        descriptor.status = RuntimeStatus::Stopped;
        descriptor.updated_at_ms = now_ms();
        self.inner.registry.upsert(descriptor.clone())?;
        Ok(descriptor)
    }

    pub async fn delete(&self, runtime_key: &str) -> Result<Option<RuntimeDescriptor>> {
        if self
            .inner
            .live_handles
            .lock()
            .await
            .contains_key(runtime_key)
        {
            self.stop(runtime_key).await?;
        }

        self.inner.registry.remove(runtime_key)
    }

    pub fn register(
        &self,
        runtime_key: &str,
        registration: RuntimeRegistration,
    ) -> Result<RuntimeDescriptor> {
        let mut descriptor = self
            .inner
            .registry
            .get(runtime_key)?
            .ok_or_else(|| anyhow!("runtime '{runtime_key}' not found"))?;

        if descriptor.status == RuntimeStatus::Stopped {
            return Err(anyhow!(
                "runtime '{runtime_key}' is stopped and cannot re-register"
            ));
        }

        let next_status = match descriptor.status {
            RuntimeStatus::Starting | RuntimeStatus::Stale => RuntimeStatus::Ready,
            RuntimeStatus::Ready => RuntimeStatus::Ready,
            RuntimeStatus::Busy => RuntimeStatus::Busy,
            RuntimeStatus::Idle => RuntimeStatus::Idle,
            RuntimeStatus::Broken => RuntimeStatus::Broken,
            RuntimeStatus::Stopped => unreachable!("stopped runtimes already returned above"),
        };

        descriptor.runtime_id = registration.runtime_id;
        descriptor.node_id = registration.node_id;
        descriptor.provider = registration.provider;
        descriptor.provider_instance_id = registration.provider_instance_id;
        descriptor.status = next_status;
        descriptor.acp = Endpoint::new(registration.advertised_acp_url);
        descriptor.state = Endpoint::new(registration.advertised_state_stream_url);
        descriptor.helper_api_base_url = registration.helper_api_base_url;
        descriptor.updated_at_ms = now_ms();
        self.inner.registry.upsert(descriptor.clone())?;
        Ok(descriptor)
    }

    pub fn heartbeat(&self, runtime_key: &str, report: HeartbeatReport) -> Result<RuntimeDescriptor> {
        let mut descriptor = self
            .inner
            .registry
            .get(runtime_key)?
            .ok_or_else(|| anyhow!("runtime '{runtime_key}' not found"))?;

        if descriptor.status == RuntimeStatus::Stopped {
            return Err(anyhow!(
                "runtime '{runtime_key}' is stopped and cannot heartbeat"
            ));
        }

        if descriptor.status == RuntimeStatus::Stale {
            descriptor.status = RuntimeStatus::Ready;
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
