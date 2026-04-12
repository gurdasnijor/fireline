use anyhow::Result;
use async_trait::async_trait;
use fireline_resources::MountedResource;
use fireline_session::ProvisionSpec;

use crate::provider::SandboxLaunch;

#[async_trait]
pub trait LocalSandboxLauncher: Send + Sync {
    async fn launch_local_sandbox(
        &self,
        spec: ProvisionSpec,
        host_key: String,
        node_id: String,
        mounted_resources: Vec<MountedResource>,
    ) -> Result<SandboxLaunch>;
}
