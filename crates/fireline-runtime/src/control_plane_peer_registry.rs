use anyhow::{Context, Result};
use async_trait::async_trait;
use fireline_tools::directory::{Peer, PeerRegistry};

use crate::runtime::{RuntimeDescriptor, RuntimeStatus};

#[derive(Clone)]
pub struct ControlPlanePeerRegistry {
    client: reqwest::Client,
    base_url: String,
}

impl ControlPlanePeerRegistry {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(2))
                .build()
                .context("build control-plane peer registry client")?,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        })
    }

    async fn fetch_runtimes(&self) -> Result<Vec<RuntimeDescriptor>> {
        self.client
            .get(format!("{}/v1/runtimes", self.base_url))
            .send()
            .await
            .context("fetch runtimes from control plane")?
            .error_for_status()
            .context("control plane rejected peer registry read")?
            .json()
            .await
            .context("decode control-plane runtime list")
    }
}

#[async_trait]
impl PeerRegistry for ControlPlanePeerRegistry {
    async fn list_peers(&self) -> Result<Vec<Peer>> {
        Ok(self
            .fetch_runtimes()
            .await?
            .into_iter()
            .filter_map(runtime_descriptor_to_peer)
            .collect())
    }

    async fn lookup_peer(&self, agent_name: &str) -> Result<Option<Peer>> {
        Ok(self
            .list_peers()
            .await?
            .into_iter()
            .find(|peer| peer.agent_name == agent_name))
    }
}

fn runtime_descriptor_to_peer(runtime: RuntimeDescriptor) -> Option<Peer> {
    if !matches!(
        runtime.status,
        RuntimeStatus::Ready | RuntimeStatus::Busy | RuntimeStatus::Idle
    ) {
        return None;
    }

    Some(Peer {
        runtime_id: runtime.runtime_id.clone(),
        agent_name: agent_name_from_runtime_id(&runtime.runtime_id),
        acp_url: runtime.acp.url,
        state_stream_url: Some(runtime.state.url),
        registered_at_ms: runtime.updated_at_ms,
    })
}

fn agent_name_from_runtime_id(runtime_id: &str) -> String {
    let mut parts = runtime_id.splitn(3, ':');
    let _prefix = parts.next();
    parts
        .next()
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| runtime_id.to_string())
}
