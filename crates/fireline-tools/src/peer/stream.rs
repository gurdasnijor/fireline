use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use durable_streams::{Client, LiveMode, Offset};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::{Notify, RwLock};
use tokio::task::JoinHandle;
use tracing::warn;

use super::{Peer, PeerRegistry};

pub const DEFAULT_TENANT_ID: &str = "default";
const DEFAULT_STALE_THRESHOLD_MS: u64 = 30_000;
const DEFAULT_POLL_INTERVAL_MS: u64 = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeploymentDiscoveryEvent {
    HostRegistered {
        host_id: String,
        acp_url: String,
        state_stream_url: String,
        capabilities: Map<String, Value>,
        registered_at_ms: i64,
        node_info: Map<String, Value>,
    },
    HostHeartbeat {
        host_id: String,
        seen_at_ms: i64,
        load_metrics: Map<String, Value>,
        runtime_count: i64,
    },
    HostDeregistered {
        host_id: String,
        reason: String,
        deregistered_at_ms: i64,
    },
    RuntimeProvisioned {
        host_id: String,
        host_key: String,
        acp_url: String,
        agent_name: String,
        provisioned_at_ms: i64,
    },
    RuntimeStopped {
        host_id: String,
        host_key: String,
        stopped_at_ms: i64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostEntry {
    pub host_id: String,
    pub acp_url: String,
    pub state_stream_url: String,
    pub capabilities: Map<String, Value>,
    pub registered_at_ms: i64,
    pub last_seen_ms: i64,
    pub last_heartbeat_metrics: Map<String, Value>,
    pub runtime_count: usize,
    pub node_info: Map<String, Value>,
    pub deregistered_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEntry {
    pub host_key: String,
    pub host_id: String,
    pub acp_url: String,
    pub agent_name: String,
    pub provisioned_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct DeploymentIndex {
    hosts: HashMap<String, HostEntry>,
    runtimes: HashMap<String, RuntimeEntry>,
    stale_threshold_ms: u64,
}

impl Default for DeploymentIndex {
    fn default() -> Self {
        Self::new(DEFAULT_STALE_THRESHOLD_MS)
    }
}

impl DeploymentIndex {
    pub fn new(stale_threshold_ms: u64) -> Self {
        Self {
            hosts: HashMap::new(),
            runtimes: HashMap::new(),
            stale_threshold_ms,
        }
    }

    pub fn apply(&mut self, event: DeploymentDiscoveryEvent) {
        match event {
            DeploymentDiscoveryEvent::HostRegistered {
                host_id,
                acp_url,
                state_stream_url,
                capabilities,
                registered_at_ms,
                node_info,
            } => {
                self.hosts.insert(
                    host_id.clone(),
                    HostEntry {
                        host_id,
                        acp_url,
                        state_stream_url,
                        capabilities,
                        registered_at_ms,
                        last_seen_ms: registered_at_ms,
                        last_heartbeat_metrics: Map::new(),
                        runtime_count: 0,
                        node_info,
                        deregistered_at_ms: None,
                    },
                );
            }
            DeploymentDiscoveryEvent::HostHeartbeat {
                host_id,
                seen_at_ms,
                load_metrics,
                runtime_count,
            } => {
                let Some(host) = self.hosts.get_mut(&host_id) else {
                    return;
                };
                host.last_seen_ms = seen_at_ms;
                host.last_heartbeat_metrics = load_metrics;
                host.runtime_count = runtime_count.max(0) as usize;
            }
            DeploymentDiscoveryEvent::HostDeregistered {
                host_id,
                deregistered_at_ms,
                ..
            } => {
                let Some(host) = self.hosts.get_mut(&host_id) else {
                    return;
                };
                host.deregistered_at_ms = Some(deregistered_at_ms);
                self.runtimes
                    .retain(|_, runtime| runtime.host_id != host_id);
            }
            DeploymentDiscoveryEvent::RuntimeProvisioned {
                host_id,
                host_key,
                acp_url,
                agent_name,
                provisioned_at_ms,
            } => {
                if !self.hosts.contains_key(&host_id) {
                    return;
                }
                self.runtimes.insert(
                    host_key.clone(),
                    RuntimeEntry {
                        host_key,
                        host_id,
                        acp_url,
                        agent_name,
                        provisioned_at_ms,
                    },
                );
            }
            DeploymentDiscoveryEvent::RuntimeStopped {
                host_id,
                host_key,
                ..
            } => {
                let Some(runtime) = self.runtimes.get(&host_key) else {
                    return;
                };
                if runtime.host_id == host_id {
                    self.runtimes.remove(&host_key);
                }
            }
        }
    }

    pub fn host(&self, host_id: &str) -> Option<&HostEntry> {
        self.hosts.get(host_id)
    }

    pub fn runtime(&self, host_key: &str) -> Option<&RuntimeEntry> {
        self.runtimes.get(host_key)
    }

    pub fn host_is_fresh(&self, host_id: &str, now_ms: i64) -> bool {
        let Some(host) = self.hosts.get(host_id) else {
            return false;
        };
        host.deregistered_at_ms.is_none()
            && now_ms.saturating_sub(host.last_seen_ms) < self.stale_threshold_ms as i64
    }

    pub fn list_fresh_runtime_peers(&self, now_ms: i64) -> Vec<Peer> {
        let mut peers: Vec<Peer> = self
            .runtimes
            .values()
            .filter(|runtime| self.host_is_fresh(&runtime.host_id, now_ms))
            .filter_map(|runtime| self.peer_for_runtime(runtime))
            .collect();
        peers.sort_by(|left, right| {
            left.agent_name
                .cmp(&right.agent_name)
                .then_with(|| left.host_id.cmp(&right.host_id))
        });
        peers
    }

    pub fn lookup_fresh_peer(&self, agent_name: &str, now_ms: i64) -> Option<Peer> {
        self.list_fresh_runtime_peers(now_ms)
            .into_iter()
            .find(|peer| peer.agent_name == agent_name)
    }

    fn peer_for_runtime(&self, runtime: &RuntimeEntry) -> Option<Peer> {
        let host = self.hosts.get(&runtime.host_id)?;
        Some(Peer {
            host_id: runtime.host_key.clone(),
            agent_name: runtime.agent_name.clone(),
            acp_url: runtime.acp_url.clone(),
            state_stream_url: Some(host.state_stream_url.clone()),
            registered_at_ms: runtime.provisioned_at_ms,
        })
    }
}

pub struct StreamDeploymentPeerRegistry {
    stream_url: String,
    tenant_id: String,
    stale_threshold_ms: u64,
    poll_interval_ms: u64,
    index: Arc<RwLock<DeploymentIndex>>,
    ready: Arc<AtomicBool>,
    ready_notify: Arc<Notify>,
    subscription: JoinHandle<()>,
}

impl StreamDeploymentPeerRegistry {
    pub fn new(stream_base_url: impl Into<String>, tenant_id: impl Into<String>) -> Self {
        Self::with_options(
            stream_base_url,
            tenant_id,
            DEFAULT_STALE_THRESHOLD_MS,
            DEFAULT_POLL_INTERVAL_MS,
        )
    }

    pub fn with_options(
        stream_base_url: impl Into<String>,
        tenant_id: impl Into<String>,
        stale_threshold_ms: u64,
        poll_interval_ms: u64,
    ) -> Self {
        let tenant_id = tenant_id.into();
        let stream_url = deployment_stream_url(&stream_base_url.into(), &tenant_id);
        let index = Arc::new(RwLock::new(DeploymentIndex::new(stale_threshold_ms)));
        let ready = Arc::new(AtomicBool::new(false));
        let ready_notify = Arc::new(Notify::new());
        let subscription = tokio::spawn(run_projection_task(
            stream_url.clone(),
            index.clone(),
            ready.clone(),
            ready_notify.clone(),
            poll_interval_ms,
        ));

        Self {
            stream_url,
            tenant_id,
            stale_threshold_ms,
            poll_interval_ms,
            index,
            ready,
            ready_notify,
            subscription,
        }
    }

    pub fn stream_url(&self) -> &str {
        &self.stream_url
    }

    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    pub fn stale_threshold_ms(&self) -> u64 {
        self.stale_threshold_ms
    }

    pub fn poll_interval_ms(&self) -> u64 {
        self.poll_interval_ms
    }

    async fn wait_until_ready(&self) {
        if self.ready.load(Ordering::SeqCst) {
            return;
        }

        let timeout = tokio::time::sleep(Duration::from_secs(2));
        tokio::pin!(timeout);
        loop {
            if self.ready.load(Ordering::SeqCst) {
                return;
            }
            tokio::select! {
                _ = self.ready_notify.notified() => {}
                _ = &mut timeout => return,
            }
        }
    }
}

impl Drop for StreamDeploymentPeerRegistry {
    fn drop(&mut self) {
        self.subscription.abort();
    }
}

#[async_trait]
impl PeerRegistry for StreamDeploymentPeerRegistry {
    async fn list_peers(&self) -> Result<Vec<Peer>> {
        self.wait_until_ready().await;
        Ok(self.index.read().await.list_fresh_runtime_peers(now_ms()))
    }

    async fn lookup_peer(&self, agent_name: &str) -> Result<Option<Peer>> {
        self.wait_until_ready().await;
        Ok(self
            .index
            .read()
            .await
            .lookup_fresh_peer(agent_name, now_ms()))
    }
}

async fn run_projection_task(
    stream_url: String,
    index: Arc<RwLock<DeploymentIndex>>,
    ready: Arc<AtomicBool>,
    ready_notify: Arc<Notify>,
    poll_interval_ms: u64,
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
            Ok(reader) => reader,
            Err(error) => {
                warn!(error = %error, stream_url, "build deployment discovery reader");
                ready.store(true, Ordering::SeqCst);
                ready_notify.notify_waiters();
                tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
                continue;
            }
        };

        loop {
            match reader.next_chunk().await {
                Ok(Some(chunk)) => {
                    if !chunk.data.is_empty() {
                        apply_chunk_bytes(&chunk.data, &index).await;
                    }

                    if chunk.up_to_date {
                        ready.store(true, Ordering::SeqCst);
                        ready_notify.notify_waiters();
                    }
                }
                Ok(None) => return,
                Err(error) => {
                    warn!(error = %error, stream_url, "deployment discovery stream read error");
                    if !error.is_retryable() {
                        ready.store(true, Ordering::SeqCst);
                        ready_notify.notify_waiters();
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
                    break;
                }
            }
        }
    }
}

async fn apply_chunk_bytes(bytes: &[u8], index: &Arc<RwLock<DeploymentIndex>>) {
    let events: Vec<Value> = match serde_json::from_slice(bytes) {
        Ok(events) => events,
        Err(error) => {
            warn!(error = %error, "deployment discovery chunk was not a JSON array");
            return;
        }
    };

    let mut index = index.write().await;
    for event in events {
        let Ok(event) = serde_json::from_value::<DeploymentDiscoveryEvent>(event) else {
            continue;
        };
        index.apply(event);
    }
}

pub fn deployment_stream_url(base_url: &str, tenant_id: &str) -> String {
    format!(
        "{}/hosts:tenant-{}",
        base_url.trim_end_matches('/'),
        tenant_id
    )
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_map() -> Map<String, Value> {
        Map::new()
    }

    #[test]
    fn host_and_runtime_become_visible_after_registration() {
        let mut index = DeploymentIndex::new(30_000);
        index.apply(DeploymentDiscoveryEvent::HostRegistered {
            host_id: "host:a".to_string(),
            acp_url: "ws://host-a/acp".to_string(),
            state_stream_url: "http://streams/v1/stream/state-a".to_string(),
            capabilities: empty_map(),
            registered_at_ms: 100,
            node_info: empty_map(),
        });
        index.apply(DeploymentDiscoveryEvent::RuntimeProvisioned {
            host_id: "host:a".to_string(),
            host_key: "runtime:alpha".to_string(),
            acp_url: "ws://host-a/acp".to_string(),
            agent_name: "alpha".to_string(),
            provisioned_at_ms: 120,
        });

        let peers = index.list_fresh_runtime_peers(130);
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].host_id, "runtime:alpha");
        assert_eq!(peers[0].agent_name, "alpha");
        assert_eq!(
            peers[0].state_stream_url.as_deref(),
            Some("http://streams/v1/stream/state-a")
        );
    }

    #[test]
    fn host_deregister_removes_hosted_runtimes() {
        let mut index = DeploymentIndex::new(30_000);
        index.apply(DeploymentDiscoveryEvent::HostRegistered {
            host_id: "host:a".to_string(),
            acp_url: "ws://host-a/acp".to_string(),
            state_stream_url: "http://streams/v1/stream/state-a".to_string(),
            capabilities: empty_map(),
            registered_at_ms: 100,
            node_info: empty_map(),
        });
        index.apply(DeploymentDiscoveryEvent::RuntimeProvisioned {
            host_id: "host:a".to_string(),
            host_key: "runtime:alpha".to_string(),
            acp_url: "ws://host-a/acp".to_string(),
            agent_name: "alpha".to_string(),
            provisioned_at_ms: 120,
        });
        index.apply(DeploymentDiscoveryEvent::HostDeregistered {
            host_id: "host:a".to_string(),
            reason: "graceful_shutdown".to_string(),
            deregistered_at_ms: 130,
        });

        assert!(index.list_fresh_runtime_peers(131).is_empty());
    }

    #[test]
    fn runtime_stop_only_affects_current_owner() {
        let mut index = DeploymentIndex::new(30_000);
        index.apply(DeploymentDiscoveryEvent::HostRegistered {
            host_id: "host:a".to_string(),
            acp_url: "ws://host-a/acp".to_string(),
            state_stream_url: "http://streams/v1/stream/state-a".to_string(),
            capabilities: empty_map(),
            registered_at_ms: 100,
            node_info: empty_map(),
        });
        index.apply(DeploymentDiscoveryEvent::HostRegistered {
            host_id: "host:b".to_string(),
            acp_url: "ws://host-b/acp".to_string(),
            state_stream_url: "http://streams/v1/stream/state-b".to_string(),
            capabilities: empty_map(),
            registered_at_ms: 100,
            node_info: empty_map(),
        });
        index.apply(DeploymentDiscoveryEvent::RuntimeProvisioned {
            host_id: "host:a".to_string(),
            host_key: "runtime:alpha".to_string(),
            acp_url: "ws://host-a/acp".to_string(),
            agent_name: "alpha".to_string(),
            provisioned_at_ms: 120,
        });
        index.apply(DeploymentDiscoveryEvent::RuntimeProvisioned {
            host_id: "host:b".to_string(),
            host_key: "runtime:alpha".to_string(),
            acp_url: "ws://host-b/acp".to_string(),
            agent_name: "alpha".to_string(),
            provisioned_at_ms: 140,
        });
        index.apply(DeploymentDiscoveryEvent::RuntimeStopped {
            host_id: "host:a".to_string(),
            host_key: "runtime:alpha".to_string(),
            stopped_at_ms: 150,
        });

        let runtime = index
            .runtime("runtime:alpha")
            .expect("runtime should remain");
        assert_eq!(runtime.host_id, "host:b");
    }

    #[test]
    fn stale_hosts_are_filtered_from_peer_listing() {
        let mut index = DeploymentIndex::new(10);
        index.apply(DeploymentDiscoveryEvent::HostRegistered {
            host_id: "host:a".to_string(),
            acp_url: "ws://host-a/acp".to_string(),
            state_stream_url: "http://streams/v1/stream/state-a".to_string(),
            capabilities: empty_map(),
            registered_at_ms: 100,
            node_info: empty_map(),
        });
        index.apply(DeploymentDiscoveryEvent::RuntimeProvisioned {
            host_id: "host:a".to_string(),
            host_key: "runtime:alpha".to_string(),
            acp_url: "ws://host-a/acp".to_string(),
            agent_name: "alpha".to_string(),
            provisioned_at_ms: 101,
        });

        assert_eq!(index.list_fresh_runtime_peers(105).len(), 1);
        assert!(index.list_fresh_runtime_peers(111).is_empty());
    }
}
