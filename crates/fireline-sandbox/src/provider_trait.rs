use anyhow::Result;
use async_trait::async_trait;
use fireline_resources::MountedResource;
use fireline_session::ProvisionSpec;

use crate::provider::RuntimeLaunch;

#[async_trait]
pub trait LocalRuntimeLauncher: Send + Sync {
    async fn launch_local_runtime(
        &self,
        spec: ProvisionSpec,
        host_key: String,
        node_id: String,
        mounted_resources: Vec<MountedResource>,
    ) -> Result<RuntimeLaunch>;
}
