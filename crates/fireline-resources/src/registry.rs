use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use durable_streams::{Client, LiveMode, Offset};
use serde_json::Value;
use tokio::sync::{RwLock, broadcast};
use tokio::task::JoinHandle;

use crate::{ResourceEntry, ResourceEvent, ResourceId, ResourceIndex};

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
    subscription: JoinHandle<()>,
}

impl StreamResourceRegistry {
    pub fn new(stream_base_url: impl Into<String>, tenant_id: impl Into<String>) -> Self {
        let tenant_id = tenant_id.into();
        let stream_url = resource_stream_url(&stream_base_url.into(), &tenant_id);
        let index = Arc::new(RwLock::new(ResourceIndex::default()));
        let (updates, _) = broadcast::channel(32);
        let subscription = tokio::spawn(run_projection_task(
            stream_url.clone(),
            index.clone(),
            updates.clone(),
        ));

        Self {
            stream_url,
            tenant_id,
            index,
            updates,
            subscription,
        }
    }

    pub fn stream_url(&self) -> &str {
        &self.stream_url
    }

    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
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
) {
    let client = Client::new();
    let stream = client.stream(&stream_url);

    let mut reader = match stream
        .read()
        .offset(Offset::Beginning)
        .live(LiveMode::Sse)
        .build()
    {
        Ok(reader) => reader,
        Err(_) => return,
    };

    loop {
        match reader.next_chunk().await {
            Ok(Some(chunk)) => {
                if chunk.data.is_empty() {
                    continue;
                }
                apply_chunk_bytes(&chunk.data, &index, &updates).await;
            }
            Ok(None) | Err(_) => return,
        }
    }
}

async fn apply_chunk_bytes(
    bytes: &[u8],
    index: &Arc<RwLock<ResourceIndex>>,
    updates: &broadcast::Sender<Vec<ResourceEntry>>,
) {
    let events: Vec<Value> = match serde_json::from_slice(bytes) {
        Ok(events) => events,
        Err(_) => return,
    };

    let mut changed = false;
    {
        let mut index = index.write().await;
        for event in events {
            let Ok(event) = serde_json::from_value::<ResourceEvent>(event) else {
                continue;
            };
            match index.apply(event) {
                Ok(applied) => changed |= applied,
                Err(_) => continue,
            }
        }

        if !changed {
            return;
        }

        let snapshot = index.list().cloned().collect::<Vec<_>>();
        let _ = updates.send(snapshot);
    }
}

fn resource_stream_url(base_url: &str, tenant_id: &str) -> String {
    format!(
        "{}/resources:tenant-{}",
        base_url.trim_end_matches('/'),
        tenant_id
    )
}
