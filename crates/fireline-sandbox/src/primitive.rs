//! The [`Sandbox`] primitive — the thing that runs a single tool call
//! in isolation inside a running session.
//!
//! This is the **Anthropic "Sandbox" primitive** in the managed-agent
//! framing: "any executor that can be configured once and called many
//! times as a tool." It is deliberately distinct from [`crate::primitives::Host`]:
//! a `Host` runs an agent *session*; a `Sandbox` runs a single *tool
//! call* (bash, code, browser, fs, …) inside that session.
//!
//! A [`crate::primitives::Host`] implementation can delegate its
//! tool-execution surface to a [`Sandbox`] impl (either a bundled one
//! or a user-provided one), or it can run tool calls inline without a
//! [`Sandbox`] at all. The two primitives compose but are not required
//! to travel together.
//!
//! This trait is the placeholder for future satisfiers
//! (`MicrosandboxSandbox`, `DockerSandbox`, `LocalProcessSandbox`, …).
//! No concrete satisfier lives in this module yet — the microsandbox
//! implementation is Tier D in
//! `docs/proposals/runtime-host-split.md` §7.5 and will land in a
//! separate commit once the trait itself is established here.

use async_trait::async_trait;
use fireline_resources::ResourceRef;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Opaque handle to a provisioned sandbox.
///
/// Satisfiers define what `id` identifies (a microsandbox name, a Docker
/// container id, a subprocess PID, …). The `kind` tag lets a shared
/// executor route handles back to the right satisfier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxHandle {
    pub id: String,
    pub kind: String,
}

impl SandboxHandle {
    pub fn new(id: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
        }
    }
}

/// A single tool call dispatched into a provisioned [`Sandbox`].
///
/// `name` identifies the tool (`bash`, `code`, `browser.navigate`, …);
/// `input` is the tool-specific payload. Deliberately minimal — the
/// primitive does not prescribe a tool registry shape. Higher-level
/// components wrap this with tool-schema validation and typed inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub name: String,
    pub input: JsonValue,
}

/// Result of a single [`Sandbox::execute`] call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallResult {
    pub output: JsonValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_status: Option<i32>,
}

/// Runs tool calls in an isolated executor.
///
/// Lifecycle is explicit: callers [`Sandbox::provision`] a handle once,
/// [`Sandbox::execute`] many calls against it, then [`Sandbox::release`]
/// the handle. Whether a sandbox is pooled per session / per turn /
/// per call is a policy decision for the caller — the trait itself
/// takes no position.
#[async_trait]
pub trait Sandbox: Send + Sync {
    /// Provision an isolated executor seeded with the given resources.
    ///
    /// The `resources` list is the same [`ResourceRef`] shape that
    /// [`crate::runtime::ResourceMounter`] consumes today — local paths,
    /// git refs, s3 / gcs prefixes. A satisfier may ignore resources
    /// it does not understand, or it may error; the contract leaves
    /// that to the satisfier.
    async fn provision(&self, resources: &[ResourceRef]) -> anyhow::Result<SandboxHandle>;

    /// Run a single tool call against a provisioned sandbox. May be
    /// called many times on the same handle before [`Sandbox::release`].
    async fn execute(
        &self,
        handle: &SandboxHandle,
        call: ToolCall,
    ) -> anyhow::Result<ToolCallResult>;

    /// Tear down the sandbox and reclaim its resources.
    async fn release(&self, handle: SandboxHandle) -> anyhow::Result<()>;
}
