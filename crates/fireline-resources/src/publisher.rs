use anyhow::Result;
use async_trait::async_trait;

use crate::{ResourceId, ResourceMetadata, ResourceMetadataPatch, ResourceSourceRef};

#[async_trait]
pub trait ResourcePublisher {
    async fn publish_resource(
        &self,
        id: ResourceId,
        source_ref: ResourceSourceRef,
        metadata: ResourceMetadata,
    ) -> Result<()>;

    async fn update_resource(&self, id: &ResourceId, patch: ResourceMetadataPatch) -> Result<()>;

    async fn unpublish_resource(&self, id: &ResourceId, reason: &str) -> Result<()>;
}
