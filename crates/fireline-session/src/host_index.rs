//! Materialized in-memory host index.
//!
//! [`HostIndex`] is the stream-derived companion to the in-memory
//! `RuntimeRegistry` living in `fireline-conductor`. It replays the
//! shared durable state stream and materializes two independent maps:
//!
//! - **`host_specs`** — keyed by `host_key`, derived from the
//!   `runtime_spec` envelopes that `emit_host_spec_persisted`
//!   writes at `crates/fireline-conductor/src/trace.rs:134`. Each row
//!   is a full [`PersistedHostSpec`] describing the originally
//!   requested host configuration.
//!
//! - **`host_instances`** — keyed by `host_id`, derived from
//!   the `runtime_instance` envelopes that every `fireline` process
//!   emits at startup (`src/bootstrap.rs:222`) and shutdown
//!   (`src/bootstrap.rs:86`). Each row carries the instance's
//!   observed `status` and timestamps.
//!
//! # Why two maps?
//!
//! The two envelope families are keyed differently:
//!
//! - `runtime_spec.key == host_key` (control-plane-assigned)
//! - `runtime_instance.key == host_id` (per-process UUID)
//!
//! They are NOT joined on the wire today. Joining them requires an
//! additional bridge — either by adding `host_key` to the
//! `runtime_instance` row, or by reading `session` rows (which carry
//! both fields via [`SessionRecord`] in the conductor crate). This
//! index stores the raw maps and exposes them separately; the
//! [`crate::host_index::tests::agreement_with_registry`] test in
//! the integration suite asserts that the stream projection agrees
//! with `RuntimeRegistry` in all observable invariants, which is the
//! empirical proof that the current wire shape is sufficient for a
//! stream-as-truth refactor.
//!
//! # Direct-host parity
//!
//! `fireline_host::bootstrap::start` — Fireline's direct-host path —
//! emits both `runtime_instance` and `runtime_spec` envelopes. Control-
//! plane-managed runtimes do the same through the sandbox host create
//! path, so the `host_specs` map now sees both launch paths.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::{PersistedHostSpec, RawStateEnvelope, HostDescriptor, StateProjection};

/// The observed lifecycle state of a single `fireline` process on
/// the shared state stream. Matches the `status` discriminator
/// serialized by `crates/fireline-conductor/src/state_projector.rs`
/// (`runtime_instance_started` / `runtime_instance_stopped`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostInstanceStatus {
    Running,
    Paused,
    Stopped,
}

/// Stream-shaped projection of a single `runtime_instance` envelope.
/// Mirrors the wire shape documented at the module level; kept as a
/// local deserializer to avoid coupling this projection to the
/// private `RuntimeInstanceRow` type inside
/// `crates/fireline-conductor/src/state_projector.rs`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInstanceRecord {
    pub instance_id: String,
    #[serde(rename = "runtimeName")]
    pub host_name: String,
    pub status: HostInstanceStatus,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct HostIndex {
    host_specs: Arc<RwLock<HashMap<String, PersistedHostSpec>>>,
    host_instances: Arc<RwLock<HashMap<String, HostInstanceRecord>>>,
    /// Latest observed `HostDescriptor` per host_key. Populated
    /// from `runtime_endpoints` envelopes emitted at every mutation
    /// point in `RuntimeHost` (create, register, stop). This is the
    /// map commits C/D of the stream-as-truth sequence will use to
    /// serve `GET /v1/runtimes` reads, replacing the in-memory
    /// `RuntimeRegistry` entirely.
    host_endpoints: Arc<RwLock<HashMap<String, HostDescriptor>>>,
}

impl HostIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the persisted spec for a given host_key, if one
    /// has been observed on the stream.
    pub async fn spec_for(&self, host_key: &str) -> Option<PersistedHostSpec> {
        self.host_specs.read().await.get(host_key).cloned()
    }

    /// Returns the list of all host_keys for which a
    /// `runtime_spec` envelope has been observed.
    pub async fn known_host_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.host_specs.read().await.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// Returns the latest observed state of a single host instance
    /// (by `host_id`), if one has been observed on the stream.
    pub async fn instance(&self, host_id: &str) -> Option<HostInstanceRecord> {
        self.host_instances.read().await.get(host_id).cloned()
    }

    /// Returns every `host_id` whose latest observed status
    /// matches the given predicate.
    pub async fn instance_ids_with_status(&self, status: HostInstanceStatus) -> Vec<String> {
        let mut matching: Vec<String> = self
            .host_instances
            .read()
            .await
            .iter()
            .filter_map(|(id, record)| (record.status == status).then(|| id.clone()))
            .collect();
        matching.sort();
        matching
    }

    /// Returns the total count of distinct host_keys observed as
    /// persisted specs plus the total count of distinct host_ids
    /// observed as instances. Used by the agreement test to shape
    /// expectations; not generally useful.
    pub async fn counts(&self) -> (usize, usize) {
        (
            self.host_specs.read().await.len(),
            self.host_instances.read().await.len(),
        )
    }

    /// Returns the latest observed `HostDescriptor` for a given
    /// host_key, derived from `runtime_endpoints` envelopes on the
    /// shared state stream. This is the replacement lookup that
    /// commit C of the stream-as-truth sequence will use in place of
    /// `RuntimeRegistry::get`.
    pub async fn endpoints_for(&self, host_key: &str) -> Option<HostDescriptor> {
        self.host_endpoints
            .read()
            .await
            .get(host_key)
            .cloned()
    }

    /// Returns all observed `HostDescriptor`s, derived from
    /// `runtime_endpoints` envelopes. Sorted by host_key for
    /// deterministic test assertions. This is the replacement for
    /// `RuntimeRegistry::list`.
    pub async fn list_endpoints(&self) -> Vec<HostDescriptor> {
        let guard = self.host_endpoints.read().await;
        let mut descriptors: Vec<HostDescriptor> = guard.values().cloned().collect();
        descriptors.sort_by(|left, right| left.host_key.cmp(&right.host_key));
        descriptors
    }

    async fn apply_envelope(&self, envelope: &RawStateEnvelope) -> Result<()> {
        match envelope.entity_type.as_str() {
            "runtime_spec" => match envelope.headers.operation.as_str() {
                "insert" | "update" => {
                    let Some(value) = envelope.value.as_ref() else {
                        return Ok(());
                    };
                    let spec: PersistedHostSpec = serde_json::from_value(value.clone())?;
                    self.host_specs
                        .write()
                        .await
                        .insert(spec.host_key.clone(), spec);
                }
                "delete" => {
                    self.host_specs.write().await.remove(&envelope.key);
                }
                _ => {}
            },
            "runtime_instance" => match envelope.headers.operation.as_str() {
                "insert" | "update" => {
                    let Some(value) = envelope.value.as_ref() else {
                        return Ok(());
                    };
                    let record: HostInstanceRecord = serde_json::from_value(value.clone())?;
                    self.host_instances
                        .write()
                        .await
                        .insert(record.instance_id.clone(), record);
                }
                "delete" => {
                    self.host_instances.write().await.remove(&envelope.key);
                }
                _ => {}
            },
            "runtime_endpoints" => match envelope.headers.operation.as_str() {
                "insert" | "update" => {
                    let Some(value) = envelope.value.as_ref() else {
                        return Ok(());
                    };
                    let descriptor: HostDescriptor = serde_json::from_value(value.clone())?;
                    self.host_endpoints
                        .write()
                        .await
                        .insert(descriptor.host_key.clone(), descriptor);
                }
                "delete" => {
                    self.host_endpoints.write().await.remove(&envelope.key);
                }
                _ => {}
            },
            _ => {}
        }

        Ok(())
    }
}

#[async_trait]
impl StateProjection for HostIndex {
    async fn apply_state_event(&self, event: &RawStateEnvelope) -> Result<()> {
        self.apply_envelope(event).await
    }

    async fn reset(&self) -> Result<()> {
        self.host_specs.write().await.clear();
        self.host_instances.write().await.clear();
        self.host_endpoints.write().await.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::path::PathBuf;

    use super::{HostIndex, HostInstanceStatus};
    use crate::{
        ProvisionSpec, PersistedHostSpec, RawStateEnvelope, SandboxProviderRequest,
        HostStatus, StateProjection, TopologySpec,
    };

    fn sample_spec(host_key: &str) -> PersistedHostSpec {
        PersistedHostSpec::new(
            host_key,
            "node:test",
            ProvisionSpec {
                host_key: None,
                node_id: None,
                provider: SandboxProviderRequest::Local,
                host: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: 0,
                name: format!("runtime-index-test-{host_key}"),
                agent_command: vec!["/bin/echo".to_string()],
                durable_streams_url: "http://127.0.0.1:8787/v1/stream".to_string(),
                resources: Vec::new(),
                state_stream: Some("state-test".to_string()),
                stream_storage: None,
                peer_directory_path: Some(PathBuf::from("/tmp/peers.toml")),
                topology: TopologySpec::default(),
            },
        )
    }

    #[tokio::test]
    async fn materializes_host_spec_rows_from_state_events() {
        let index = HostIndex::new();
        let host_spec = sample_spec("runtime:one");
        let envelope: RawStateEnvelope = serde_json::from_value(serde_json::json!({
            "type": "runtime_spec",
            "key": "runtime:one",
            "headers": { "operation": "insert" },
            "value": host_spec,
        }))
        .unwrap();

        index.apply_state_event(&envelope).await.unwrap();

        let fetched = index.spec_for("runtime:one").await.expect("spec indexed");
        assert_eq!(fetched.host_key, "runtime:one");
        assert_eq!(index.known_host_keys().await, vec!["runtime:one"]);
    }

    #[tokio::test]
    async fn materializes_runtime_instance_rows_from_state_events() {
        let index = HostIndex::new();
        let envelope: RawStateEnvelope = serde_json::from_value(serde_json::json!({
            "type": "runtime_instance",
            "key": "fireline:one:abcd",
            "headers": { "operation": "insert" },
            "value": {
                "instanceId": "fireline:one:abcd",
                "runtimeName": "one",
                "status": "running",
                "createdAt": 100,
                "updatedAt": 100,
            }
        }))
        .unwrap();

        index.apply_state_event(&envelope).await.unwrap();

        let record = index
            .instance("fireline:one:abcd")
            .await
            .expect("instance indexed");
        assert_eq!(record.status, HostInstanceStatus::Running);
        assert_eq!(
            index
                .instance_ids_with_status(HostInstanceStatus::Running)
                .await,
            vec!["fireline:one:abcd".to_string()]
        );
    }

    #[tokio::test]
    async fn running_to_stopped_transition_is_observable() {
        let index = HostIndex::new();

        let started: RawStateEnvelope = serde_json::from_value(serde_json::json!({
            "type": "runtime_instance",
            "key": "fireline:one:abcd",
            "headers": { "operation": "insert" },
            "value": {
                "instanceId": "fireline:one:abcd",
                "runtimeName": "one",
                "status": "running",
                "createdAt": 100,
                "updatedAt": 100,
            }
        }))
        .unwrap();
        let stopped: RawStateEnvelope = serde_json::from_value(serde_json::json!({
            "type": "runtime_instance",
            "key": "fireline:one:abcd",
            "headers": { "operation": "update" },
            "value": {
                "instanceId": "fireline:one:abcd",
                "runtimeName": "one",
                "status": "stopped",
                "createdAt": 100,
                "updatedAt": 200,
            }
        }))
        .unwrap();

        index.apply_state_event(&started).await.unwrap();
        index.apply_state_event(&stopped).await.unwrap();

        let record = index.instance("fireline:one:abcd").await.unwrap();
        assert_eq!(record.status, HostInstanceStatus::Stopped);
        assert_eq!(record.updated_at, 200);
        assert!(
            index
                .instance_ids_with_status(HostInstanceStatus::Running)
                .await
                .is_empty()
        );
        assert_eq!(
            index
                .instance_ids_with_status(HostInstanceStatus::Stopped)
                .await,
            vec!["fireline:one:abcd".to_string()]
        );
    }

    #[tokio::test]
    async fn reset_clears_both_maps() {
        let index = HostIndex::new();
        let spec_envelope: RawStateEnvelope = serde_json::from_value(serde_json::json!({
            "type": "runtime_spec",
            "key": "runtime:one",
            "headers": { "operation": "insert" },
            "value": sample_spec("runtime:one"),
        }))
        .unwrap();
        let instance_envelope: RawStateEnvelope = serde_json::from_value(serde_json::json!({
            "type": "runtime_instance",
            "key": "fireline:one:abcd",
            "headers": { "operation": "insert" },
            "value": {
                "instanceId": "fireline:one:abcd",
                "runtimeName": "one",
                "status": "running",
                "createdAt": 100,
                "updatedAt": 100,
            }
        }))
        .unwrap();

        index.apply_state_event(&spec_envelope).await.unwrap();
        index.apply_state_event(&instance_envelope).await.unwrap();
        assert_eq!(index.counts().await, (1, 1));

        StateProjection::reset(&index).await.unwrap();
        assert_eq!(index.counts().await, (0, 0));
    }

    #[tokio::test]
    async fn materializes_host_endpoints_rows_from_state_events() {
        let index = HostIndex::new();
        let envelope: RawStateEnvelope = serde_json::from_value(serde_json::json!({
            "type": "runtime_endpoints",
            "key": "runtime:one",
            "headers": { "operation": "update" },
            "value": {
                "runtimeKey": "runtime:one",
                "runtimeId": "fireline:one:abcd",
                "nodeId": "node:test",
                "provider": "local",
                "providerInstanceId": "local:1",
                "status": "ready",
                "acp": { "url": "ws://127.0.0.1:9991/acp" },
                "state": { "url": "http://127.0.0.1:9991/v1/stream/state-one" },
                "createdAtMs": 100,
                "updatedAtMs": 200
            }
        }))
        .unwrap();

        index.apply_state_event(&envelope).await.unwrap();

        let descriptor = index
            .endpoints_for("runtime:one")
            .await
            .expect("endpoints indexed");
        assert_eq!(descriptor.host_key, "runtime:one");
        assert_eq!(descriptor.host_id, "fireline:one:abcd");
        assert_eq!(descriptor.acp.url, "ws://127.0.0.1:9991/acp");
        assert_eq!(
            descriptor.state.url,
            "http://127.0.0.1:9991/v1/stream/state-one"
        );

        let listed = index.list_endpoints().await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].host_key, "runtime:one");
    }

    #[tokio::test]
    async fn endpoints_update_overwrites_previous_observation() {
        let index = HostIndex::new();
        let first: RawStateEnvelope = serde_json::from_value(serde_json::json!({
            "type": "runtime_endpoints",
            "key": "runtime:one",
            "headers": { "operation": "update" },
            "value": {
                "runtimeKey": "runtime:one",
                "runtimeId": "fireline:one:abcd",
                "nodeId": "node:test",
                "provider": "local",
                "providerInstanceId": "local:1",
                "status": "ready",
                "acp": { "url": "ws://127.0.0.1:9991/acp" },
                "state": { "url": "http://127.0.0.1:9991/v1/stream/state-one" },
                "createdAtMs": 100,
                "updatedAtMs": 200
            }
        }))
        .unwrap();
        let after_stop: RawStateEnvelope = serde_json::from_value(serde_json::json!({
            "type": "runtime_endpoints",
            "key": "runtime:one",
            "headers": { "operation": "update" },
            "value": {
                "runtimeKey": "runtime:one",
                "runtimeId": "fireline:one:abcd",
                "nodeId": "node:test",
                "provider": "local",
                "providerInstanceId": "local:1",
                "status": "stopped",
                "acp": { "url": "ws://127.0.0.1:9991/acp" },
                "state": { "url": "http://127.0.0.1:9991/v1/stream/state-one" },
                "createdAtMs": 100,
                "updatedAtMs": 300
            }
        }))
        .unwrap();

        index.apply_state_event(&first).await.unwrap();
        index.apply_state_event(&after_stop).await.unwrap();

        let descriptor = index.endpoints_for("runtime:one").await.unwrap();
        assert_eq!(descriptor.updated_at_ms, 300);
        assert!(matches!(descriptor.status, HostStatus::Stopped));
    }

    #[tokio::test]
    async fn unknown_entity_types_are_ignored() {
        let index = HostIndex::new();
        let envelope: RawStateEnvelope = serde_json::from_value(serde_json::json!({
            "type": "session",
            "key": "sess-1",
            "headers": { "operation": "insert" },
            "value": { "sessionId": "sess-1" }
        }))
        .unwrap();

        index.apply_state_event(&envelope).await.unwrap();
        assert_eq!(index.counts().await, (0, 0));
    }
}
