//! The [`Host`] primitive — the thing that runs an agent session.
//!
//! A `Host` owns session lifecycle and exposes the idempotent, retry-safe
//! `wake` verb that drives a session one step forward. See
//! [`crate::primitives`] for the cross-primitive overview and
//! `docs/proposals/client-primitives.md` §Module 2 for the canonical
//! TypeScript contract.
//!
//! The one concrete satisfier in this module is [`FirelineHost`], a thin
//! wrapper around [`crate::runtime::RuntimeHost`] that maps the
//! session-level verbs onto Fireline's runtime-level lifecycle. Additional
//! satisfiers (Claude Agent SDK v2, microsandbox-hosted runtimes, …) will
//! live in separate modules or crates and implement [`Host`] against the
//! same contract.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::runtime::{
    CreateRuntimeSpec, ResourceRef, RuntimeHost, RuntimeProviderRequest, RuntimeStatus,
};
use crate::topology::TopologySpec;

//--------------------------------------------------------------------------------------------------
// Core types
//--------------------------------------------------------------------------------------------------

/// Opaque handle to a session created by a [`Host`].
///
/// Satisfiers define what `id` actually identifies: for [`FirelineHost`]
/// it is the `runtime_key` of the managed runtime that backs the session;
/// for a future Claude-host satisfier it will be Claude's session id
/// returned from the `query({ resume })` flow. The `kind` tag carries the
/// host discriminator so a shared orchestrator can route handles back to
/// the right satisfier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHandle {
    pub id: String,
    pub kind: String,
}

impl SessionHandle {
    pub fn new(id: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
        }
    }
}

/// Observed status of a session. Mirrors the TS
/// `SessionStatus` discriminated union at `client-primitives.md` §Module 2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionStatus {
    Created,
    Running,
    Idle,
    NeedsWake,
    Stopped,
    Error { message: String },
}

/// Result of a [`Host::wake`] call. Orchestrators use this to decide
/// whether to keep pumping or back off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WakeOutcome {
    /// Nothing to do — session is up to date.
    Noop,
    /// Session was advanced by `steps` logical steps.
    Advanced { steps: u64 },
    /// Session is blocked on an external condition; orchestrator should
    /// stop pumping until that condition is cleared.
    Blocked { reason: String },
}

/// Declarative session creation payload.
///
/// This is a *union* of host-specific needs — each [`Host`] satisfier
/// honors the fields it understands and ignores the rest. `FirelineHost`
/// consumes `topology` / `resources` / `agent_command` / `name`; a future
/// Claude-host satisfier would consume `model` / `initial_prompt` /
/// `metadata` and ignore the Fireline-specific fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topology: Option<TopologySpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<ResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_command: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, JsonValue>,
}

//--------------------------------------------------------------------------------------------------
// The trait
//--------------------------------------------------------------------------------------------------

/// Runs agent sessions.
///
/// Every method must be safe to call from multiple tasks concurrently.
/// [`Host::wake`] in particular MUST be idempotent and retry-safe:
/// calling it multiple times with the same [`SessionHandle`] — even
/// concurrently — must converge on the same session state without
/// replaying side effects.
///
/// Streaming input (the optional `sendInput` verb in the TypeScript
/// contract) is deliberately out of scope for the v1 Rust trait. Hosts
/// that want live stdin/stdout streaming should expose their own
/// higher-level API next to this trait until there is a concrete second
/// satisfier that validates the shape.
#[async_trait]
pub trait Host: Send + Sync {
    /// Reserve (or provision) a session identifier.
    ///
    /// For [`FirelineHost`] this provisions a managed runtime via the
    /// control plane and waits for the runtime to reach `Ready`. For a
    /// future Claude-host satisfier this would allocate a session id
    /// and append an initial record to the durable stream without
    /// round-tripping to Claude.
    async fn create_session(&self, spec: SessionSpec) -> anyhow::Result<SessionHandle>;

    /// Advance the session one logical step.
    ///
    /// Must be idempotent and retry-safe. Implementations that are
    /// purely event-driven (e.g. [`FirelineHost`], which advances via
    /// ACP messages on a live WebSocket) return [`WakeOutcome::Noop`];
    /// wake-polled hosts (e.g. a future Claude-host satisfier driving
    /// `query({ resume })`) return [`WakeOutcome::Advanced`] with the
    /// number of steps they drained.
    async fn wake(&self, handle: &SessionHandle) -> anyhow::Result<WakeOutcome>;

    /// Observational — never mutates. Orchestrators call this to decide
    /// whether to call [`Host::wake`] again.
    async fn status(&self, handle: &SessionHandle) -> anyhow::Result<SessionStatus>;

    /// Tear down the session's execution state. Does *not* delete the
    /// durable log: subsequent [`Host::create_session`] calls targeting
    /// the same logical session (if the host supports resume) should
    /// rebuild from the log.
    async fn stop_session(&self, handle: &SessionHandle) -> anyhow::Result<()>;
}

//--------------------------------------------------------------------------------------------------
// FirelineHost — the native satisfier
//--------------------------------------------------------------------------------------------------

/// The `kind` tag carried by every [`SessionHandle`] that [`FirelineHost`]
/// returns. Useful for orchestrators routing handles across multiple
/// [`Host`] satisfiers.
pub const FIRELINE_HOST_KIND: &str = "fireline";

/// [`Host`] satisfier that wraps Fireline's native [`RuntimeHost`].
///
/// A "session" at the [`Host`] primitive layer corresponds to a managed
/// runtime at the [`crate::runtime`] layer — one runtime backs one
/// session. [`Host::create_session`] delegates to
/// [`RuntimeHost::create`], [`Host::status`] reads the runtime descriptor
/// from the registry, and [`Host::stop_session`] delegates to
/// [`RuntimeHost::stop`]. [`Host::wake`] is a no-op because Fireline
/// sessions are advanced live via ACP messages on a WebSocket, not
/// pulled through a wake verb. A future iteration that drains durable
/// pending-input rows via a projection would flip `wake` to return
/// [`WakeOutcome::Advanced`] when it finds pending work.
#[derive(Clone)]
pub struct FirelineHost {
    runtime_host: RuntimeHost,
}

impl FirelineHost {
    pub fn new(runtime_host: RuntimeHost) -> Self {
        Self { runtime_host }
    }

    /// Reference to the underlying [`RuntimeHost`] for callers that need
    /// to reach lower-level runtime operations (list, delete, heartbeat,
    /// register) not yet covered by the [`Host`] trait.
    pub fn runtime_host(&self) -> &RuntimeHost {
        &self.runtime_host
    }
}

#[async_trait]
impl Host for FirelineHost {
    async fn create_session(&self, spec: SessionSpec) -> anyhow::Result<SessionHandle> {
        let create_spec = CreateRuntimeSpec {
            runtime_key: None,
            node_id: None,
            provider: RuntimeProviderRequest::Auto,
            host: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            name: spec
                .name
                .unwrap_or_else(|| "fireline-session".to_string()),
            agent_command: spec.agent_command.unwrap_or_default(),
            resources: spec.resources,
            state_stream: None,
            stream_storage: None,
            peer_directory_path: None,
            topology: spec.topology.unwrap_or(TopologySpec {
                components: Vec::new(),
            }),
        };
        let descriptor = self.runtime_host.create(create_spec).await?;
        Ok(SessionHandle::new(descriptor.runtime_key, FIRELINE_HOST_KIND))
    }

    async fn wake(&self, _handle: &SessionHandle) -> anyhow::Result<WakeOutcome> {
        Ok(WakeOutcome::Noop)
    }

    async fn status(&self, handle: &SessionHandle) -> anyhow::Result<SessionStatus> {
        let descriptor = self.runtime_host.get(&handle.id)?;
        Ok(match descriptor {
            None => SessionStatus::Error {
                message: format!("session '{}' not found", handle.id),
            },
            Some(d) => match d.status {
                RuntimeStatus::Starting => SessionStatus::Created,
                RuntimeStatus::Ready | RuntimeStatus::Busy => SessionStatus::Running,
                RuntimeStatus::Idle | RuntimeStatus::Stale => SessionStatus::Idle,
                RuntimeStatus::Stopped => SessionStatus::Stopped,
                RuntimeStatus::Broken => SessionStatus::Error {
                    message: format!("runtime '{}' is broken", handle.id),
                },
            },
        })
    }

    async fn stop_session(&self, handle: &SessionHandle) -> anyhow::Result<()> {
        self.runtime_host.stop(&handle.id).await?;
        Ok(())
    }
}
