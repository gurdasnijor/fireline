use std::collections::HashMap;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{HostId, ResourceSourceRef};

pub type ResourceId = String;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ResourceMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, Value>,
}

impl ResourceMetadata {
    pub fn apply_patch(&mut self, patch: ResourceMetadataPatch) {
        if let Some(size_bytes) = patch.size_bytes {
            self.size_bytes = Some(size_bytes);
        }
        if let Some(mime_type) = patch.mime_type {
            self.mime_type = Some(mime_type);
        }
        if let Some(content_hash) = patch.content_hash {
            self.content_hash = Some(content_hash);
        }
        if let Some(tags) = patch.tags {
            self.tags = tags;
        }
        if let Some(permissions) = patch.permissions {
            self.permissions = Some(permissions);
        }
        if let Some(description) = patch.description {
            self.description = Some(description);
        }
        for (key, value) in patch.extra {
            self.extra.insert(key, value);
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ResourceMetadataPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResourceEvent {
    ResourcePublished(ResourcePublishedEvent),
    ResourceUnpublished(ResourceUnpublishedEvent),
    ResourceUpdated(ResourceUpdatedEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourcePublishedEvent {
    pub resource_id: ResourceId,
    pub source_ref: ResourceSourceRef,
    pub metadata: ResourceMetadata,
    pub published_by: HostId,
    pub published_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceUnpublishedEvent {
    pub resource_id: ResourceId,
    pub reason: String,
    pub unpublished_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceUpdatedEvent {
    pub resource_id: ResourceId,
    pub new_metadata: ResourceMetadataPatch,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceEntry {
    pub resource_id: ResourceId,
    pub source_ref: ResourceSourceRef,
    pub metadata: ResourceMetadata,
    pub published_by: HostId,
    pub first_seen_ms: i64,
    pub last_updated_ms: i64,
}

#[derive(Debug, Clone, Default)]
pub struct ResourceIndex {
    resources: HashMap<ResourceId, ResourceEntry>,
}

impl ResourceIndex {
    pub fn lookup(&self, id: &ResourceId) -> Option<&ResourceEntry> {
        self.resources.get(id)
    }

    pub fn list(&self) -> impl Iterator<Item = &ResourceEntry> {
        self.resources.values()
    }

    pub fn list_by_tag(&self, tag: &str) -> impl Iterator<Item = &ResourceEntry> {
        self.resources
            .values()
            .filter(move |entry| entry.metadata.tags.iter().any(|entry_tag| entry_tag == tag))
    }

    pub fn apply(&mut self, event: ResourceEvent) -> Result<bool> {
        match event {
            ResourceEvent::ResourcePublished(event) => self.apply_published(event),
            ResourceEvent::ResourceUnpublished(event) => Ok(self.resources.remove(&event.resource_id).is_some()),
            ResourceEvent::ResourceUpdated(event) => self.apply_updated(event),
        }
    }

    fn apply_published(&mut self, event: ResourcePublishedEvent) -> Result<bool> {
        if let Some(existing) = self.resources.get(&event.resource_id) {
            let duplicate = existing.source_ref == event.source_ref
                && existing.metadata == event.metadata
                && existing.published_by == event.published_by
                && existing.first_seen_ms == event.published_at_ms
                && existing.last_updated_ms == event.published_at_ms;
            if duplicate {
                return Ok(false);
            }
            return Err(anyhow!(
                "resource '{}' was already published with a different source or publisher",
                event.resource_id
            ));
        }

        self.resources.insert(
            event.resource_id.clone(),
            ResourceEntry {
                resource_id: event.resource_id,
                source_ref: event.source_ref,
                metadata: event.metadata,
                published_by: event.published_by,
                first_seen_ms: event.published_at_ms,
                last_updated_ms: event.published_at_ms,
            },
        );
        Ok(true)
    }

    fn apply_updated(&mut self, event: ResourceUpdatedEvent) -> Result<bool> {
        let Some(existing) = self.resources.get_mut(&event.resource_id) else {
            return Ok(false);
        };

        existing.metadata.apply_patch(event.new_metadata);
        existing.last_updated_ms = event.updated_at_ms;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::{
        ResourceEvent, ResourceIndex, ResourceMetadata, ResourceMetadataPatch,
        ResourcePublishedEvent, ResourceUnpublishedEvent, ResourceUpdatedEvent,
    };
    use crate::ResourceSourceRef;

    fn published_event() -> ResourcePublishedEvent {
        ResourcePublishedEvent {
            resource_id: "resource-1".to_string(),
            source_ref: ResourceSourceRef::DurableStreamBlob {
                stream: "resources:tenant-demo".to_string(),
                key: "blob-1".to_string(),
            },
            metadata: ResourceMetadata {
                tags: vec!["dataset".to_string()],
                ..ResourceMetadata::default()
            },
            published_by: "host-a".to_string(),
            published_at_ms: 100,
        }
    }

    #[test]
    fn index_projects_publish_update_and_unpublish() {
        let mut index = ResourceIndex::default();

        index
            .apply(ResourceEvent::ResourcePublished(published_event()))
            .unwrap();
        assert!(index.lookup(&"resource-1".to_string()).is_some());
        assert_eq!(index.list_by_tag("dataset").count(), 1);

        index
            .apply(ResourceEvent::ResourceUpdated(ResourceUpdatedEvent {
                resource_id: "resource-1".to_string(),
                new_metadata: ResourceMetadataPatch {
                    description: Some("indexed".to_string()),
                    extra: HashMap::from([("format".to_string(), json!("parquet"))]),
                    ..ResourceMetadataPatch::default()
                },
                updated_at_ms: 200,
            }))
            .unwrap();

        let entry = index.lookup(&"resource-1".to_string()).unwrap();
        assert_eq!(entry.metadata.description.as_deref(), Some("indexed"));
        assert_eq!(entry.metadata.extra.get("format"), Some(&json!("parquet")));
        assert_eq!(entry.last_updated_ms, 200);

        index
            .apply(ResourceEvent::ResourceUnpublished(ResourceUnpublishedEvent {
                resource_id: "resource-1".to_string(),
                reason: "cleanup".to_string(),
                unpublished_at_ms: 300,
            }))
            .unwrap();
        assert!(index.lookup(&"resource-1".to_string()).is_none());
    }

    #[test]
    fn duplicate_publish_is_a_noop_when_the_payload_matches() {
        let mut index = ResourceIndex::default();
        let event = published_event();

        assert!(index
            .apply(ResourceEvent::ResourcePublished(event.clone()))
            .unwrap());
        assert!(!index
            .apply(ResourceEvent::ResourcePublished(event))
            .unwrap());
    }
}
