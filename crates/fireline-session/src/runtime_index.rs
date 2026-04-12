//! Materialized in-memory runtime index.
//!
//! [`RuntimeIndex`] is the stream-derived companion to the in-memory
//! `RuntimeRegistry` living in `fireline-conductor`. It replays the
//! shared durable state stream and materializes two independent maps:
//!
//! - **`runtime_specs`** — keyed by `runtime_key`, derived from the
//!   `runtime_spec` envelopes that `emit_runtime_spec_persisted`
//!   writes at `crates/fireline-conductor/src/trace.rs:134`. Each row
//!   is a full [`PersistedRuntimeSpec`] describing the originally
//!   requested runtime configuration.
//!
//! - **`runtime_instances`** — keyed by `runtime_id`, derived from
//!   the `runtime_instance` envelopes that every `fireline` process
//!   emits at startup (`src/bootstrap.rs:222`) and shutdown
//!   (`src/bootstrap.rs:86`). Each row carries the instance's
//!   observed `status` and timestamps.
//!
//! # Why two maps?
//!
//! The two envelope families are keyed differently:
//!
//! - `runtime_spec.key == runtime_key` (control-plane-assigned)
//! - `runtime_instance.key == runtime_id` (per-process UUID)
//!
//! They are NOT joined on the wire today. Joining them requires an
//! additional bridge — either by adding `runtime_key` to the
//! `runtime_instance` row, or by reading `session` rows (which carry
//! both fields via [`SessionRecord`] in the conductor crate). This
//! index stores the raw maps and exposes them separately; the
//! [`crate::runtime_index::tests::agreement_with_registry`] test in
//! the integration suite asserts that the stream projection agrees
//! with `RuntimeRegistry` in all observable invariants, which is the
//! empirical proof that the current wire shape is sufficient for a
//! stream-as-truth refactor.
//!
//! # Known gap: direct-host mode
//!
//! `src/bootstrap.rs::start` — Fireline's direct-host path — emits
//! `runtime_instance` events but NOT `runtime_spec`. Control-plane-
//! managed runtimes go through `RuntimeHost::create`, which emits both.
//! So the `runtime_specs` map only sees control-plane-managed
//! runtimes today. This is documented and known; closing it is a
//! prerequisite for fully replacing `RuntimeRegistry` with a
//! stream-derived view.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::{PersistedRuntimeSpec, RawStateEnvelope, RuntimeDescriptor, StateProjection};

/// The observed lifecycle state of a single `fireline` process on
/// the shared state stream. Matches the `status` discriminator
/// serialized by `crates/fireline-conductor/src/state_projector.rs`
/// (`runtime_instance_started` / `runtime_instance_stopped`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeInstanceStatus {
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
pub struct RuntimeInstanceRecord {
    pub instance_id: String,
    pub runtime_name: String,
    pub status: RuntimeInstanceStatus,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeIndex {
    runtime_specs: Arc<RwLock<HashMap<String, PersistedRuntimeSpec>>>,
    runtime_instances: Arc<RwLock<HashMap<String, RuntimeInstanceRecord>>>,
    /// Latest observed `RuntimeDescriptor` per runtime_key. Populated
    /// from `runtime_endpoints` envelopes emitted at every mutation
    /// point in `RuntimeHost` (create, register, stop). This is the
    /// map commits C/D of the stream-as-truth sequence will use to
    /// serve `GET /v1/runtimes` reads, replacing the in-memory
    /// `RuntimeRegistry` entirely.
    runtime_endpoints: Arc<RwLock<HashMap<String, RuntimeDescriptor>>>,
}

impl RuntimeIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the persisted spec for a given runtime_key, if one
    /// has been observed on the stream.
    pub async fn spec_for(&self, runtime_key: &str) -> Option<PersistedRuntimeSpec> {
        self.runtime_specs.read().await.get(runtime_key).cloned()
    }

    /// Returns the list of all runtime_keys for which a
    /// `runtime_spec` envelope has been observed.
    pub async fn known_runtime_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.runtime_specs.read().await.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// Returns the latest observed state of a single runtime instance
    /// (by `runtime_id`), if one has been observed on the stream.
    pub async fn instance(&self, runtime_id: &str) -> Option<RuntimeInstanceRecord> {
        self.runtime_instances.read().await.get(runtime_id).cloned()
    }

    /// Returns every `runtime_id` whose latest observed status
    /// matches the given predicate.
    pub async fn instance_ids_with_status(&self, status: RuntimeInstanceStatus) -> Vec<String> {
        let mut matching: Vec<String> = self
            .runtime_instances
            .read()
            .await
            .iter()
            .filter_map(|(id, record)| (record.status == status).then(|| id.clone()))
            .collect();
        matching.sort();
        matching
    }

    /// Returns the total count of distinct runtime_keys observed as
    /// persisted specs plus the total count of distinct runtime_ids
    /// observed as instances. Used by the agreement test to shape
    /// expectations; not generally useful.
    pub async fn counts(&self) -> (usize, usize) {
        (
            self.runtime_specs.read().await.len(),
            self.runtime_instances.read().await.len(),
        )
    }

    /// Returns the latest observed `RuntimeDescriptor` for a given
    /// runtime_key, derived from `runtime_endpoints` envelopes on the
    /// shared state stream. This is the replacement lookup that
    /// commit C of the stream-as-truth sequence will use in place of
    /// `RuntimeRegistry::get`.
    pub async fn endpoints_for(&self, runtime_key: &str) -> Option<RuntimeDescriptor> {
        self.runtime_endpoints
            .read()
            .await
            .get(runtime_key)
            .cloned()
    }

    /// Returns all observed `RuntimeDescriptor`s, derived from
    /// `runtime_endpoints` envelopes. Sorted by runtime_key for
    /// deterministic test assertions. This is the replacement for
    /// `RuntimeRegistry::list`.
    pub async fn list_endpoints(&self) -> Vec<RuntimeDescriptor> {
        let guard = self.runtime_endpoints.read().await;
        let mut descriptors: Vec<RuntimeDescriptor> = guard.values().cloned().collect();
        descriptors.sort_by(|left, right| left.runtime_key.cmp(&right.runtime_key));
        descriptors
    }

    async fn apply_envelope(&self, envelope: &RawStateEnvelope) -> Result<()> {
        match envelope.entity_type.as_str() {
            "runtime_spec" => match envelope.headers.operation.as_str() {
                "insert" | "update" => {
                    let Some(value) = envelope.value.as_ref() else {
                        return Ok(());
                    };
                    let spec: PersistedRuntimeSpec = serde_json::from_value(value.clone())?;
                    self.runtime_specs
                        .write()
                        .await
                        .insert(spec.runtime_key.clone(), spec);
                }
                "delete" => {
                    self.runtime_specs.write().await.remove(&envelope.key);
                }
                _ => {}
            },
            "runtime_instance" => match envelope.headers.operation.as_str() {
                "insert" | "update" => {
                    let Some(value) = envelope.value.as_ref() else {
                        return Ok(());
                    };
                    let record: RuntimeInstanceRecord = serde_json::from_value(value.clone())?;
                    self.runtime_instances
                        .write()
                        .await
                        .insert(record.instance_id.clone(), record);
                }
                "delete" => {
                    self.runtime_instances.write().await.remove(&envelope.key);
                }
                _ => {}
            },
            "runtime_endpoints" => match envelope.headers.operation.as_str() {
                "insert" | "update" => {
                    let Some(value) = envelope.value.as_ref() else {
                        return Ok(());
                    };
                    let descriptor: RuntimeDescriptor = serde_json::from_value(value.clone())?;
                    self.runtime_endpoints
                        .write()
                        .await
                        .insert(descriptor.runtime_key.clone(), descriptor);
                }
                "delete" => {
                    self.runtime_endpoints.write().await.remove(&envelope.key);
                }
                _ => {}
            },
            _ => {}
        }

        Ok(())
    }
}

#[async_trait]
impl StateProjection for RuntimeIndex {
    async fn apply_state_event(&self, event: &RawStateEnvelope) -> Result<()> {
        self.apply_envelope(event).await
    }

    async fn reset(&self) -> Result<()> {
        self.runtime_specs.write().await.clear();
        self.runtime_instances.write().await.clear();
        self.runtime_endpoints.write().await.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::path::PathBuf;

    use super::{RuntimeIndex, RuntimeInstanceStatus};
    use crate::{
        CreateRuntimeSpec, PersistedRuntimeSpec, RawStateEnvelope, RuntimeProviderRequest,
        RuntimeStatus, StateProjection, TopologySpec,
    };

    fn sample_spec(runtime_key: &str) -> PersistedRuntimeSpec {
        PersistedRuntimeSpec::new(
            runtime_key,
            "node:test",
            CreateRuntimeSpec {
                runtime_key: None,
                node_id: None,
                provider: RuntimeProviderRequest::Local,
                host: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: 0,
                name: format!("runtime-index-test-{runtime_key}"),
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
    async fn materializes_runtime_spec_rows_from_state_events() {
        let index = RuntimeIndex::new();
        let runtime_spec = sample_spec("runtime:one");
        let envelope: RawStateEnvelope = serde_json::from_value(serde_json::json!({
            "type": "runtime_spec",
            "key": "runtime:one",
            "headers": { "operation": "insert" },
            "value": runtime_spec,
        }))
        .unwrap();

        index.apply_state_event(&envelope).await.unwrap();

        let fetched = index.spec_for("runtime:one").await.expect("spec indexed");
        assert_eq!(fetched.runtime_key, "runtime:one");
        assert_eq!(index.known_runtime_keys().await, vec!["runtime:one"]);
    }

    #[tokio::test]
    async fn materializes_runtime_instance_rows_from_state_events() {
        let index = RuntimeIndex::new();
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
        assert_eq!(record.status, RuntimeInstanceStatus::Running);
        assert_eq!(
            index
                .instance_ids_with_status(RuntimeInstanceStatus::Running)
                .await,
            vec!["fireline:one:abcd".to_string()]
        );
    }

    #[tokio::test]
    async fn running_to_stopped_transition_is_observable() {
        let index = RuntimeIndex::new();

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
        assert_eq!(record.status, RuntimeInstanceStatus::Stopped);
        assert_eq!(record.updated_at, 200);
        assert!(
            index
                .instance_ids_with_status(RuntimeInstanceStatus::Running)
                .await
                .is_empty()
        );
        assert_eq!(
            index
                .instance_ids_with_status(RuntimeInstanceStatus::Stopped)
                .await,
            vec!["fireline:one:abcd".to_string()]
        );
    }

    #[tokio::test]
    async fn reset_clears_both_maps() {
        let index = RuntimeIndex::new();
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
    async fn materializes_runtime_endpoints_rows_from_state_events() {
        let index = RuntimeIndex::new();
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
        assert_eq!(descriptor.runtime_key, "runtime:one");
        assert_eq!(descriptor.runtime_id, "fireline:one:abcd");
        assert_eq!(descriptor.acp.url, "ws://127.0.0.1:9991/acp");
        assert_eq!(
            descriptor.state.url,
            "http://127.0.0.1:9991/v1/stream/state-one"
        );

        let listed = index.list_endpoints().await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].runtime_key, "runtime:one");
    }

    #[tokio::test]
    async fn endpoints_update_overwrites_previous_observation() {
        let index = RuntimeIndex::new();
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
        assert!(matches!(descriptor.status, RuntimeStatus::Stopped));
    }

    #[tokio::test]
    async fn unknown_entity_types_are_ignored() {
        let index = RuntimeIndex::new();
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
