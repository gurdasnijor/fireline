//! [`Sandbox`] satisfier backed by the [`microsandbox`] crate.
//!
//! Boots a hardware-isolated microVM per provisioned sandbox via
//! [`microsandbox::Sandbox::create_detached`], mounts caller-supplied
//! [`ResourceRef::LocalPath`] resources as bind volumes, runs tool
//! calls via [`microsandbox::Sandbox::exec_stream`], and releases the
//! VM on [`Sandbox::release`].
//!
//! See `docs/proposals/runtime-host-split.md` §7.4 for why this is a
//! [`Sandbox`] and not a [`fireline_conductor::primitives::Host`]
//! satisfier, and the reconnaissance spikes at
//! `/tmp/fireline-microsandbox-spike/` (not in-repo) for the
//! boot-time and networking findings this implementation relies on.
//!
//! ## Tool-call convention
//!
//! The minimal v1 contract: every [`ToolCall::name`] must be either
//! `"shell"` (runs `call.input["command"]` as a `/bin/sh -c` script,
//! returning combined stdout) or `"exec"` (runs
//! `call.input["argv"]: [String, ..]` as a direct exec, returning
//! combined stdout). Richer tool shapes — `browser`, `code`,
//! tool-specific JSON schemas — are deliberately out of scope until a
//! `Host` satisfier actually consumes this [`Sandbox`] trait and the
//! shape of the host→sandbox bridge is empirically validated.
//!
//! ## Resource handling
//!
//! The v1 implementation understands [`ResourceRef::LocalPath`] and
//! turns each into a `.volume(guest_path, |m| m.bind(host_path))`
//! builder call. Other [`ResourceRef`] variants (`GitRemote`, `S3`,
//! `Gcs`) are currently rejected with a descriptive error — mirroring
//! how [`crate::fs_backend`] handles the unsupported cases — rather
//! than silently dropped. Adding them is a follow-up aligned with
//! [`fireline_conductor::runtime::ResourceMounter`] symmetry.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use fireline_conductor::primitives::{Sandbox, SandboxHandle, ToolCall, ToolCallResult};
use fireline_conductor::runtime::ResourceRef;
use microsandbox::{NetworkPolicy, Sandbox as MsbSandbox};
use serde_json::{Value as JsonValue, json};
use tokio::sync::Mutex;
use uuid::Uuid;

/// The `kind` tag every [`SandboxHandle`] returned by
/// [`MicrosandboxSandbox::provision`] carries. Shared executors use
/// this to route handles back to the right satisfier.
pub const MICROSANDBOX_SANDBOX_KIND: &str = "microsandbox";

/// Static configuration for a [`MicrosandboxSandbox`].
///
/// Held on the satisfier itself, applied verbatim to every
/// provisioned VM. Per-provision overrides (a specific image for a
/// single tool call, for example) are not yet supported — the
/// expectation is one [`MicrosandboxSandbox`] value per distinct
/// "pool" of tool executors, and a `Host` satisfier picks the pool
/// at tool-dispatch time.
#[derive(Debug, Clone)]
pub struct MicrosandboxSandboxConfig {
    /// OCI image or local rootfs reference, e.g. `"alpine"` or
    /// `"python"`. Passed verbatim to [`microsandbox::Sandbox::builder`]
    /// via `.image(...)`.
    pub image: String,

    /// Guest vCPU count. Default: 1.
    pub cpus: u8,

    /// Guest memory in MiB. Default: 512.
    pub memory_mib: u32,

    /// Optional hard lifetime cap for every provisioned sandbox. Maps
    /// to [`microsandbox::Sandbox::builder`]'s `.max_duration(...)`.
    pub max_duration_secs: Option<u64>,

    /// Optional idle-shutdown timeout. Maps to
    /// [`microsandbox::Sandbox::builder`]'s `.idle_timeout(...)`.
    pub idle_timeout_secs: Option<u64>,
}

impl MicrosandboxSandboxConfig {
    pub fn alpine() -> Self {
        Self {
            image: "alpine".to_string(),
            cpus: 1,
            memory_mib: 512,
            max_duration_secs: None,
            idle_timeout_secs: None,
        }
    }
}

impl Default for MicrosandboxSandboxConfig {
    fn default() -> Self {
        Self::alpine()
    }
}

/// [`Sandbox`] satisfier that boots a microsandbox microVM per
/// provisioned handle and runs tool calls inside it.
pub struct MicrosandboxSandbox {
    config: MicrosandboxSandboxConfig,
    // Provisioned VMs, keyed by the opaque SandboxHandle id we hand
    // back to callers. The handle id doubles as the microsandbox
    // sandbox name (sanitized and uuid-suffixed at provision time).
    live: Arc<Mutex<HashMap<String, MsbSandbox>>>,
}

impl MicrosandboxSandbox {
    pub fn new(config: MicrosandboxSandboxConfig) -> Self {
        Self {
            config,
            live: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn config(&self) -> &MicrosandboxSandboxConfig {
        &self.config
    }
}

#[async_trait]
impl Sandbox for MicrosandboxSandbox {
    async fn provision(&self, resources: &[ResourceRef]) -> Result<SandboxHandle> {
        let id = format!("fireline-ms-{}", Uuid::new_v4());

        let mut builder = MsbSandbox::builder(&id)
            .image(self.config.image.as_str())
            .cpus(self.config.cpus)
            .memory(self.config.memory_mib)
            // allow_all is intentional: §7.4 of runtime-host-split.md
            // documents why RFC1918-blocking policies break guest
            // egress in our use case. A narrower non_local() policy
            // is a good follow-up once a Host satisfier is actually
            // consuming this trait.
            .network(|n| n.policy(NetworkPolicy::allow_all()))
            .replace();

        if let Some(secs) = self.config.max_duration_secs {
            builder = builder.max_duration(secs);
        }
        if let Some(secs) = self.config.idle_timeout_secs {
            builder = builder.idle_timeout(secs);
        }

        for resource in resources {
            match resource {
                ResourceRef::LocalPath { path, mount_path } => {
                    let guest = mount_path.to_string_lossy().into_owned();
                    let host = path.clone();
                    builder = builder.volume(guest, move |m| m.bind(host));
                }
                ResourceRef::GitRemote { .. } => {
                    return Err(anyhow!(
                        "MicrosandboxSandbox: ResourceRef::GitRemote is not yet supported; \
                         pre-clone the repo on the host and pass it as a LocalPath instead",
                    ));
                }
                ResourceRef::S3 { .. } => {
                    return Err(anyhow!(
                        "MicrosandboxSandbox: ResourceRef::S3 is not yet supported",
                    ));
                }
                ResourceRef::Gcs { .. } => {
                    return Err(anyhow!(
                        "MicrosandboxSandbox: ResourceRef::Gcs is not yet supported",
                    ));
                }
            }
        }

        let sandbox = builder
            .create_detached()
            .await
            .with_context(|| format!("microsandbox::Sandbox::create_detached('{id}')"))?;

        self.live.lock().await.insert(id.clone(), sandbox);
        Ok(SandboxHandle::new(id, MICROSANDBOX_SANDBOX_KIND))
    }

    async fn execute(
        &self,
        handle: &SandboxHandle,
        call: ToolCall,
    ) -> Result<ToolCallResult> {
        let guard = self.live.lock().await;
        let sandbox = guard.get(&handle.id).ok_or_else(|| {
            anyhow!(
                "MicrosandboxSandbox: no provisioned sandbox for handle '{}'",
                handle.id
            )
        })?;

        match call.name.as_str() {
            "shell" => {
                let command = call
                    .input
                    .get("command")
                    .and_then(JsonValue::as_str)
                    .ok_or_else(|| {
                        anyhow!("ToolCall 'shell' requires input.command to be a string")
                    })?;
                let output = sandbox
                    .shell(command.to_string())
                    .await
                    .with_context(|| format!("microsandbox shell '{command}'"))?;
                let stdout = output.stdout().unwrap_or_default();
                let stderr = output.stderr().unwrap_or_default();
                let exit = output.status().code;
                Ok(ToolCallResult {
                    output: json!({ "stdout": stdout, "stderr": stderr }),
                    exit_status: Some(exit),
                })
            }
            "exec" => {
                let argv = call
                    .input
                    .get("argv")
                    .and_then(JsonValue::as_array)
                    .ok_or_else(|| anyhow!("ToolCall 'exec' requires input.argv to be an array"))?;
                let argv: Vec<String> = argv
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| anyhow!("ToolCall 'exec' argv entries must be strings"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let (head, rest) = argv
                    .split_first()
                    .ok_or_else(|| anyhow!("ToolCall 'exec' argv must be non-empty"))?;
                // exec() takes a command string; we stringify the rest
                // of argv as positional args by joining with spaces.
                // This is the v1 simplification — once a Host satisfier
                // is wiring real tool calls we'll plumb a proper argv
                // struct through to microsandbox's exec_with builder.
                let cmd = if rest.is_empty() {
                    head.clone()
                } else {
                    format!("{head} {}", rest.join(" "))
                };
                let output = sandbox
                    .exec(cmd.clone(), Vec::<String>::new())
                    .await
                    .with_context(|| format!("microsandbox exec '{cmd}'"))?;
                let stdout = output.stdout().unwrap_or_default();
                let stderr = output.stderr().unwrap_or_default();
                let exit = output.status().code;
                Ok(ToolCallResult {
                    output: json!({ "stdout": stdout, "stderr": stderr }),
                    exit_status: Some(exit),
                })
            }
            other => Err(anyhow!(
                "MicrosandboxSandbox: tool '{other}' is not supported yet; \
                 use 'shell' or 'exec'",
            )),
        }
    }

    async fn release(&self, handle: SandboxHandle) -> Result<()> {
        let sandbox = {
            let mut guard = self.live.lock().await;
            guard.remove(&handle.id)
        };
        let Some(sandbox) = sandbox else {
            return Err(anyhow!(
                "MicrosandboxSandbox: no provisioned sandbox for handle '{}'",
                handle.id
            ));
        };
        // Best-effort graceful stop followed by best-effort persisted-
        // state removal. We don't propagate remove_persisted errors
        // because once the VM is stopped the caller has already seen
        // the release semantics they need.
        let _ = sandbox
            .stop_and_wait()
            .await
            .with_context(|| format!("microsandbox stop_and_wait '{}'", handle.id));
        let _ = sandbox
            .remove_persisted()
            .await
            .with_context(|| format!("microsandbox remove_persisted '{}'", handle.id));
        Ok(())
    }
}
