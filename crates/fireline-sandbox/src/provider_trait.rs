use anyhow::Result;
use async_trait::async_trait;
use fireline_resources::MountedResource;
use fireline_session::CreateRuntimeSpec;

use crate::provider::RuntimeLaunch;

#[async_trait]
pub trait LocalRuntimeLauncher: Send + Sync {
    async fn launch_local_runtime(
        &self,
        spec: CreateRuntimeSpec,
        runtime_key: String,
        node_id: String,
        mounted_resources: Vec<MountedResource>,
    ) -> Result<RuntimeLaunch>;
}
