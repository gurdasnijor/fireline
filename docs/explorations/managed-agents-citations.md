# Managed Agents Citations

> **SUPERSEDED** by [`../proposals/client-api-redesign.md`](../proposals/client-api-redesign.md) and [`../proposals/sandbox-provider-model.md`](../proposals/sandbox-provider-model.md). Retained for architectural history.

Source primitive set: Anthropic, ["Scaling Managed Agents: Decoupling the brain from the hands"](https://www.anthropic.com/engineering/managed-agents).  
Code snapshot: local `main` at `a4dc19c`.

## 1. Session — Strong

- Closest code today:
  - `crates/fireline-conductor/src/trace.rs` — producer-side append path from ACP/conductor effects into durable streams.
  - `crates/fireline-conductor/src/state_projector.rs` — projects durable trace events into typed `session`, `prompt_turn`, `permission`, and `chunk` rows.
  - `src/runtime_materializer.rs` and `src/session_index.rs` — replay/live-follow plus `session_id -> SessionRecord` lookup.
  - `packages/state/src/collection.ts`, `packages/state/src/schema.ts`, `packages/client/src/browser.ts` — browser/read-model side consuming the same replayable stream.

- Relevant signatures:
```rust
// src/runtime_materializer.rs
#[async_trait]
pub trait StateProjection: Send + Sync {
    async fn apply_state_event(&self, event: &RawStateEnvelope) -> Result<()>;
}

pub struct RuntimeMaterializer;
impl RuntimeMaterializer {
    pub fn new(projections: Vec<Arc<dyn StateProjection>>) -> Self;
    pub fn connect(&self, state_stream_url: impl Into<String>) -> RuntimeMaterializerTask;
}

// src/session_index.rs
pub struct SessionIndex;
impl SessionIndex {
    pub fn new() -> Self;
    pub async fn get(&self, session_id: &str) -> Option<SessionRecord>;
    pub async fn list(&self) -> Vec<SessionRecord>;
}
```

- Live contract coverage:
  - `tests/managed_agent_session.rs:52` — append-only replay from offset 0
  - `tests/managed_agent_session.rs:126` — durability across runtime death
  - `tests/managed_agent_session.rs:213` — replay from a captured mid-stream offset
  - `tests/managed_agent_session.rs:415` — idempotent append under retry
  - `tests/managed_agent_session.rs:523` — materialized-vs-raw agreement

- Assessment: Strong. The durable log, replay surface, retry-safe producer semantics, and materialized consumer agreement all have live managed-agent proofs.

## 2. Orchestration — Strong

- Closest code today:
  - `src/orchestration.rs:12-73` — `resume(session_id)` composition helper over shared session state, runtime lookup, cold-start, and readiness polling.
  - `src/orchestration.rs:75-108` — `reconstruct_runtime_spec_from_log(...)` and materialization helpers.
  - `src/load_coordinator.rs` — `session/load` rebuild seam.
  - `crates/fireline-conductor/src/runtime/mod.rs` and `src/runtime_host.rs` — runtime lifecycle and readiness.
  - `crates/fireline-control-plane/src/router.rs` — runtime create/get/stop surfaces orchestration composes against.

- Relevant signatures:
```rust
// src/orchestration.rs
pub async fn resume(
    http: &HttpClient,
    control_plane_url: &str,
    shared_state_url: &str,
    session_id: &str,
) -> Result<RuntimeDescriptor>;

pub async fn reconstruct_runtime_spec_from_log(
    state_stream_url: &str,
    runtime_key: &str,
) -> Result<PersistedRuntimeSpec>;
```

- Live contract coverage:
  - `tests/managed_agent_primitives_suite.rs:132` — cold-start orchestration acceptance contract
  - `tests/managed_agent_orchestration.rs:84` — `resume` is a no-op on a live runtime
  - `tests/managed_agent_orchestration.rs:161` — concurrent `resume` calls converge on one runtime
  - `tests/managed_agent_orchestration.rs:249` — subscriber loop drives pause-release through the durable stream

- Assessment: Strong on the Rust-side primitive. The remaining gap is API ownership in `@fireline/client`, not missing orchestration substrate.

## 3. Harness — Strong

- Closest code today:
  - `crates/fireline-conductor/src/build.rs` — conductor composition around the underlying agent/harness.
  - `src/routes/acp.rs` — runtime-owned ACP entrypoint where the harness is exposed.
  - `crates/fireline-conductor/src/shared_terminal.rs` — long-lived subprocess plus attach/detach lifecycle.
  - `crates/fireline-conductor/src/trace.rs` — durable logging of harness progress.

- Relevant signatures:
```rust
// crates/fireline-conductor/src/build.rs
pub fn build_subprocess_conductor(...) -> ConductorImpl<Agent>;
pub fn build_conductor_with_terminal(...) -> ConductorImpl<Agent>;

// crates/fireline-conductor/src/shared_terminal.rs
pub struct SharedTerminal;
impl SharedTerminal {
    pub async fn spawn(agent_command: Vec<String>) -> Result<Self>;
    pub async fn try_attach(&self) -> Result<SharedTerminalAttachment, AttachError>;
    pub async fn shutdown(&self) -> Result<()>;
}
```

- Live contract coverage:
  - `tests/managed_agent_harness.rs:64` — every effect is appended to the durable session log
  - `tests/managed_agent_harness.rs:139` — append order remains stable under continued writes
  - `tests/managed_agent_harness.rs:248` — approval-gate prompt blocks until durable resolution
  - `tests/managed_agent_harness.rs:333` — durable suspend/resume survives runtime death
  - `tests/managed_agent_harness.rs:477` — all seven topology combinators are represented in the harness surface

- Assessment: Strong. Fireline now has live proofs for durable progress logging, stable append order, suspend/release through the stream, and suspend/resume across runtime death.

## 4. Sandbox — Strong

- Closest code today:
  - `crates/fireline-conductor/src/runtime/provider.rs:130-173` — `CreateRuntimeSpec`, `PersistedRuntimeSpec`, `RuntimeProvider`, and `RuntimeLaunch`.
  - `src/runtime_provider.rs` and `src/bootstrap.rs` — local runtime provisioning.
  - `crates/fireline-control-plane/src/local_provider.rs` — control-plane-backed subprocess provisioning and shutdown path.

- Relevant signatures:
```rust
// crates/fireline-conductor/src/runtime/provider.rs
pub struct CreateRuntimeSpec {
    pub provider: RuntimeProviderRequest,
    pub agent_command: Vec<String>,
    pub resources: Vec<ResourceRef>,
    pub state_stream: Option<String>,
    pub topology: TopologySpec,
    ...
}

pub trait RuntimeProvider: Send + Sync {
    fn kind(&self) -> RuntimeProviderKind;
    async fn start(&self, spec: CreateRuntimeSpec, runtime_key: String, node_id: String)
        -> Result<RuntimeLaunch>;
}
```

- Live contract coverage:
  - `tests/managed_agent_sandbox.rs:58` — provision returns a reachable runtime
  - `tests/managed_agent_sandbox.rs:109` — a provisioned runtime serves multiple execute calls
  - `tests/managed_agent_sandbox.rs:185` — stop + recreate preserves `session/load`
  - `tests/managed_agent_sandbox.rs:267` — cross-provider equivalence is intentionally delegated to `tests/control_plane_docker.rs` slice 13c via an explicit cross-reference marker

- Assessment: Strong. The local/control-plane-backed Sandbox contract is live, and the Docker mixed-topology proof is explicitly tracked outside the lightweight managed-agent suite.

## 5. Resources — Strong

- Closest code today:
  - `crates/fireline-conductor/src/runtime/provider.rs:132-148` — `CreateRuntimeSpec.resources`.
  - `crates/fireline-conductor/src/runtime/mounter.rs:10-89` — `MountedResource`, `ResourceMounter`, `LocalPathMounter`, and `prepare_resources(...)`.
  - `crates/fireline-components/src/fs_backend.rs:23-152` — `FileBackend` plus `FsBackendComponent`.
  - `crates/fireline-components/src/fs_backend.rs:155-260` — `LocalFileBackend` and `RuntimeStreamFileBackend` / `SessionLogFileBackend`.

- Relevant signatures:
```rust
// crates/fireline-conductor/src/runtime/mounter.rs
pub struct MountedResource {
    pub host_path: PathBuf,
    pub mount_path: PathBuf,
    pub read_only: bool,
}

#[async_trait]
pub trait ResourceMounter: Send + Sync {
    async fn mount(&self, resource: &ResourceRef, runtime_key: &str)
        -> Result<Option<MountedResource>>;
}

// crates/fireline-components/src/fs_backend.rs
#[async_trait]
pub trait FileBackend: Send + Sync {
    async fn read(&self, path: &Path) -> Result<Vec<u8>>;
    async fn write(&self, path: &Path, content: &[u8]) -> Result<()>;
}

pub struct FsBackendComponent;
```

- Live contract coverage:
  - `tests/managed_agent_resources.rs:60` — `LocalPathMounter` maps source to mount correctly
  - `tests/managed_agent_resources.rs:108` — `LocalFileBackend` reads through mount mapping
  - `tests/managed_agent_resources.rs:195` — launched runtime captures ACP fs writes as durable `fs_op`
  - `tests/managed_agent_resources.rs:287` — stream-backed backend supports cross-runtime reads
  - `tests/managed_agent_primitives_suite.rs:249` — acceptance-level physical mount contract at component layer
  - `tests/managed_agent_primitives_suite.rs:308` — acceptance-level fs-backend contract
  - `tests/managed_agent_primitives_suite.rs:369` — component-level fs-backend evidence path
  - `tests/managed_agent_resources.rs:154` and `tests/managed_agent_primitives_suite.rs:290` — shell-visible mount remains an intentional Docker-scoped cross-reference marker, not missing local-runtime substrate

- Assessment: Strong. Local-path mounting, ACP fs interception, artifact capture, and cross-runtime stream-backed file reads all have live coverage. The only non-local-runtime proof is the intentionally external Docker/container shell-visible mount invariant.

## 6. Tools — Strong

- Closest code today:
  - `crates/fireline-components/src/peer/mcp_server.rs:57-84` — canonical `ToolDescriptor` generation for peer tools.
  - `crates/fireline-components/src/tools.rs:1-195` — schema-only `ToolDescriptor`, `TransportRef`, `CredentialRef`, `CapabilityRef`, and durable `tool_descriptor` emission.
  - `crates/fireline-components/src/smithery.rs` — Smithery-backed tool attachment path.

- Relevant signatures:
```rust
// crates/fireline-components/src/tools.rs
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub enum TransportRef {
    PeerRuntime { runtime_key: String },
    Smithery { catalog: String, tool: String },
    McpUrl { url: String },
    InProcess { component_name: String },
}

pub struct CapabilityRef {
    pub descriptor: ToolDescriptor,
    pub transport_ref: TransportRef,
    pub credential_ref: Option<CredentialRef>,
}
```

- Live contract coverage:
  - `tests/managed_agent_tools.rs:63` — schema-only descriptor surface with no transport/credential leakage
  - `tests/managed_agent_tools.rs:230` — transport-agnostic registration projects the same wire shape
  - `tests/managed_agent_tools.rs:389` — same-name collisions resolve deterministically via first-attach-wins
  - `tests/managed_agent_primitives_suite.rs:430` — acceptance-level schema-only sibling in the primitives suite

- Assessment: Strong. The Anthropic tool triple is live and enforced against real topology wiring; Slice 17 now extends portability and credential indirection rather than fixing a missing primitive.
