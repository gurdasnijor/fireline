use anyhow::{Context, Result};
use async_trait::async_trait;
use durable_streams::{Client as DurableStreamsClient, CreateOptions};
use uuid::Uuid;

use crate::{
    HostId, ResourceEvent, ResourceId, ResourceMetadata, ResourceMetadataPatch,
    ResourcePublishedEvent, ResourceSourceRef, ResourceUnpublishedEvent, ResourceUpdatedEvent,
};

#[derive(Debug, Clone)]
pub struct StreamResourcePublisher {
    stream_url: String,
    published_by: HostId,
}

impl StreamResourcePublisher {
    pub fn new(
        stream_base_url: impl Into<String>,
        tenant_id: impl AsRef<str>,
        published_by: impl Into<HostId>,
    ) -> Self {
        Self {
            stream_url: resource_stream_url(&stream_base_url.into(), tenant_id.as_ref()),
            published_by: published_by.into(),
        }
    }

    pub fn stream_url(&self) -> &str {
        &self.stream_url
    }
}

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

#[async_trait]
impl ResourcePublisher for StreamResourcePublisher {
    async fn publish_resource(
        &self,
        id: ResourceId,
        source_ref: ResourceSourceRef,
        metadata: ResourceMetadata,
    ) -> Result<()> {
        append_event(
            &self.stream_url,
            &format!("resource-published-{}", sanitize_producer_component(&id)),
            &ResourceEvent::ResourcePublished(ResourcePublishedEvent {
                resource_id: id,
                source_ref,
                metadata,
                published_by: self.published_by.clone(),
                published_at_ms: now_ms(),
            }),
        )
        .await
    }

    async fn update_resource(&self, id: &ResourceId, patch: ResourceMetadataPatch) -> Result<()> {
        append_event(
            &self.stream_url,
            &format!("resource-updated-{}", sanitize_producer_component(id)),
            &ResourceEvent::ResourceUpdated(ResourceUpdatedEvent {
                resource_id: id.clone(),
                new_metadata: patch,
                updated_at_ms: now_ms(),
            }),
        )
        .await
    }

    async fn unpublish_resource(&self, id: &ResourceId, reason: &str) -> Result<()> {
        append_event(
            &self.stream_url,
            &format!("resource-unpublished-{}", sanitize_producer_component(id)),
            &ResourceEvent::ResourceUnpublished(ResourceUnpublishedEvent {
                resource_id: id.clone(),
                reason: reason.to_string(),
                unpublished_at_ms: now_ms(),
            }),
        )
        .await
    }
}

async fn append_event(stream_url: &str, producer_prefix: &str, event: &ResourceEvent) -> Result<()> {
    ensure_json_stream_exists(stream_url).await?;

    let client = DurableStreamsClient::new();
    let mut stream = client.stream(stream_url);
    stream.set_content_type("application/json");
    let producer = stream
        .producer(format!("{producer_prefix}-{}", Uuid::new_v4()))
        .content_type("application/json")
        .build();
    producer.append_json(event);
    producer.flush().await?;
    Ok(())
}

async fn ensure_json_stream_exists(stream_url: &str) -> Result<()> {
    let client = DurableStreamsClient::new();
    let stream = client.stream(stream_url);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match stream
            .create_with(CreateOptions::new().content_type("application/json"))
            .await
        {
            Ok(_) => return Ok(()),
            Err(error) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                if matches!(error, durable_streams::StreamError::Conflict) {
                    return Err(anyhow::Error::from(error))
                        .with_context(|| format!("create resource stream '{stream_url}'"));
                }
            }
            Err(error) => {
                return Err(anyhow::Error::from(error))
                    .with_context(|| format!("create resource stream '{stream_url}'"));
            }
        }
    }
}

fn resource_stream_url(base_url: &str, tenant_id: &str) -> String {
    format!(
        "{}/resources:tenant-{}",
        base_url.trim_end_matches('/'),
        tenant_id
    )
}

fn sanitize_producer_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => ch,
            _ => '-',
        })
        .collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}
