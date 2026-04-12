use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use durable_streams::{Client, LiveMode, Offset};
use serde_json::Value;
use tokio::sync::{RwLock, broadcast};
use tokio::task::JoinHandle;
use tracing::warn;

use crate::{ResourceEntry, ResourceEvent, ResourceId, ResourceIndex};

const PROJECTION_RETRY_DELAY_MS: u64 = 1_000;

#[async_trait]
pub trait ResourceWatcher: Send + Sync {
    async fn on_index_updated(&self, entries: Vec<ResourceEntry>) -> Result<()>;
}

pub struct Subscription {
    handle: JoinHandle<()>,
}

impl Subscription {
    pub fn new(handle: JoinHandle<()>) -> Self {
        Self { handle }
    }

    pub fn abort(self) {
        self.handle.abort();
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

#[async_trait]
pub trait ResourceRegistry: Send + Sync {
    async fn lookup(&self, id: &ResourceId) -> Result<Option<ResourceEntry>>;
    async fn list(&self) -> Result<Vec<ResourceEntry>>;
    async fn list_by_tag(&self, tag: &str) -> Result<Vec<ResourceEntry>>;
    async fn subscribe(&self, watcher: Box<dyn ResourceWatcher>) -> Result<Subscription>;
}

pub struct StreamResourceRegistry {
    stream_url: String,
    tenant_id: String,
    index: Arc<RwLock<ResourceIndex>>,
    updates: broadcast::Sender<Vec<ResourceEntry>>,
    projection_healthy: Arc<AtomicBool>,
    projection_error_count: Arc<AtomicU64>,
    subscription: JoinHandle<()>,
}

impl StreamResourceRegistry {
    pub fn new(stream_base_url: impl Into<String>, tenant_id: impl Into<String>) -> Self {
        let tenant_id = tenant_id.into();
        let stream_url = resource_stream_url(&stream_base_url.into(), &tenant_id);
        let index = Arc::new(RwLock::new(ResourceIndex::default()));
        let (updates, _) = broadcast::channel(32);
        let projection_healthy = Arc::new(AtomicBool::new(true));
        let projection_error_count = Arc::new(AtomicU64::new(0));
        let subscription = tokio::spawn(run_projection_task(
            stream_url.clone(),
            index.clone(),
            updates.clone(),
            projection_healthy.clone(),
            projection_error_count.clone(),
        ));

        Self {
            stream_url,
            tenant_id,
            index,
            updates,
            projection_healthy,
            projection_error_count,
            subscription,
        }
    }

    pub fn stream_url(&self) -> &str {
        &self.stream_url
    }

    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    pub fn is_healthy(&self) -> bool {
        self.projection_healthy.load(Ordering::SeqCst)
    }

    pub fn projection_error_count(&self) -> u64 {
        self.projection_error_count.load(Ordering::SeqCst)
    }
}

impl Drop for StreamResourceRegistry {
    fn drop(&mut self) {
        self.subscription.abort();
    }
}

#[async_trait]
impl ResourceRegistry for StreamResourceRegistry {
    async fn lookup(&self, id: &ResourceId) -> Result<Option<ResourceEntry>> {
        Ok(self.index.read().await.lookup(id).cloned())
    }

    async fn list(&self) -> Result<Vec<ResourceEntry>> {
        Ok(self.index.read().await.list().cloned().collect())
    }

    async fn list_by_tag(&self, tag: &str) -> Result<Vec<ResourceEntry>> {
        Ok(self.index.read().await.list_by_tag(tag).cloned().collect())
    }

    async fn subscribe(&self, watcher: Box<dyn ResourceWatcher>) -> Result<Subscription> {
        watcher.on_index_updated(self.list().await?).await?;

        let mut receiver = self.updates.subscribe();
        let handle = tokio::spawn(async move {
            while let Ok(entries) = receiver.recv().await {
                if watcher.on_index_updated(entries).await.is_err() {
                    break;
                }
            }
        });

        Ok(Subscription::new(handle))
    }
}

async fn run_projection_task(
    stream_url: String,
    index: Arc<RwLock<ResourceIndex>>,
    updates: broadcast::Sender<Vec<ResourceEntry>>,
    projection_healthy: Arc<AtomicBool>,
    projection_error_count: Arc<AtomicU64>,
) {
    let client = Client::new();
    let stream = client.stream(&stream_url);

    loop {
        let mut reader = match stream
            .read()
            .offset(Offset::Beginning)
            .live(LiveMode::Sse)
            .build()
        {
            Ok(reader) => {
                projection_healthy.store(true, Ordering::SeqCst);
                reader
            }
            Err(error) => {
                projection_healthy.store(false, Ordering::SeqCst);
                projection_error_count.fetch_add(1, Ordering::SeqCst);
                warn!(error = %error, stream_url = %stream_url, "build resource registry reader");
                tokio::time::sleep(Duration::from_millis(PROJECTION_RETRY_DELAY_MS)).await;
                continue;
            }
        };

        loop {
            match reader.next_chunk().await {
                Ok(Some(chunk)) => {
                    if chunk.data.is_empty() {
                        continue;
                    }
                    apply_chunk_bytes(
                        &stream_url,
                        &chunk.data,
                        &index,
                        &updates,
                        &projection_error_count,
                    )
                    .await;
                }
                Ok(None) => {
                    projection_healthy.store(false, Ordering::SeqCst);
                    projection_error_count.fetch_add(1, Ordering::SeqCst);
                    warn!(stream_url = %stream_url, "resource registry stream closed");
                    return;
                }
                Err(error) => {
                    projection_healthy.store(false, Ordering::SeqCst);
                    projection_error_count.fetch_add(1, Ordering::SeqCst);
                    warn!(
                        error = %error,
                        stream_url = %stream_url,
                        "resource registry stream read error"
                    );
                    if !error.is_retryable() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(PROJECTION_RETRY_DELAY_MS)).await;
                    break;
                }
            }
        }
    }
}

async fn apply_chunk_bytes(
    stream_url: &str,
    bytes: &[u8],
    index: &Arc<RwLock<ResourceIndex>>,
    updates: &broadcast::Sender<Vec<ResourceEntry>>,
    projection_error_count: &Arc<AtomicU64>,
) {
    let events: Vec<Value> = match serde_json::from_slice(bytes) {
        Ok(events) => events,
        Err(error) => {
            projection_error_count.fetch_add(1, Ordering::SeqCst);
            warn!(
                error = %error,
                stream_url = %stream_url,
                chunk_size = bytes.len(),
                "resource registry chunk was not a JSON array"
            );
            return;
        }
    };

    let mut changed = false;
    {
        let mut index = index.write().await;
        for event in events {
            let event_type = event_type_name(&event).unwrap_or("unknown").to_string();
            let resource_id = event_resource_id(&event).unwrap_or("unknown").to_string();
            let Ok(event) = serde_json::from_value::<ResourceEvent>(event) else {
                projection_error_count.fetch_add(1, Ordering::SeqCst);
                warn!(
                    stream_url = %stream_url,
                    event_type = %event_type,
                    resource_id = %resource_id,
                    "resource registry event did not match ResourceEvent schema"
                );
                continue;
            };
            let event_type = resource_event_type(&event);
            let resource_id = resource_event_id(&event).to_string();
            match index.apply(event) {
                Ok(applied) => changed |= applied,
                Err(error) => {
                    projection_error_count.fetch_add(1, Ordering::SeqCst);
                    warn!(
                        error = %error,
                        stream_url = %stream_url,
                        event_type,
                        resource_id = %resource_id,
                        "resource registry failed to apply event"
                    );
                    continue;
                }
            }
        }

        if !changed {
            return;
        }

        let snapshot = index.list().cloned().collect::<Vec<_>>();
        let _ = updates.send(snapshot);
    }
}

fn event_type_name(event: &Value) -> Option<&str> {
    event.get("type").and_then(Value::as_str)
}

fn event_resource_id(event: &Value) -> Option<&str> {
    event.get("resource_id").and_then(Value::as_str)
}

fn resource_event_type(event: &ResourceEvent) -> &'static str {
    match event {
        ResourceEvent::ResourcePublished(_) => "resource_published",
        ResourceEvent::ResourceUnpublished(_) => "resource_unpublished",
        ResourceEvent::ResourceUpdated(_) => "resource_updated",
    }
}

fn resource_event_id(event: &ResourceEvent) -> &str {
    match event {
        ResourceEvent::ResourcePublished(event) => &event.resource_id,
        ResourceEvent::ResourceUnpublished(event) => &event.resource_id,
        ResourceEvent::ResourceUpdated(event) => &event.resource_id,
    }
}

fn resource_stream_url(base_url: &str, tenant_id: &str) -> String {
    format!(
        "{}/resources:tenant-{}",
        base_url.trim_end_matches('/'),
        tenant_id
    )
}
