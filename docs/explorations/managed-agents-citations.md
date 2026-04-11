# Managed Agents Citations

Source primitive set: Anthropic, ["Scaling Managed Agents: Decoupling the brain from the hands"](https://www.anthropic.com/engineering/managed-agents).  
Code snapshot: local `main` at `47bc10b`.

## 1. Session — Strong

- Closest code today:
  - `crates/fireline-conductor/src/trace.rs:43-89` — `DurableStreamTracer` is the producer-side append path from ACP/conductor events into the durable stream.
  - `crates/fireline-conductor/src/state_projector.rs:189-249,389-419,478-565` — projects those trace events into durable `session`, `prompt_turn`, `pending_request`, and `chunk` rows.
  - `src/runtime_materializer.rs:34-83,90-133` and `src/session_index.rs:19-67` — replay/live-follow plus `session_id -> SessionRecord` lookup.
  - `packages/state/src/collection.ts:35-55`, `packages/state/src/schema.ts:91-193`, `packages/client/src/browser.ts:33-67` — browser/read-model side of the same replayable stream.

- Relevant signatures:
```rust
// src/runtime_materializer.rs:34-54
#[async_trait]
pub trait StateProjection: Send + Sync {
    async fn apply_state_event(&self, event: &RawStateEnvelope) -> Result<()>;
}
pub struct RuntimeMaterializer;
impl RuntimeMaterializer {
    pub fn new(projections: Vec<Arc<dyn StateProjection>>) -> Self;
    pub fn connect(&self, state_stream_url: impl Into<String>) -> RuntimeMaterializerTask;
}

// src/session_index.rs:19-33
pub struct SessionIndex;
impl SessionIndex {
    pub fn new() -> Self;
    pub async fn get(&self, session_id: &str) -> Option<SessionRecord>;
    pub async fn list(&self) -> Vec<SessionRecord>;
}
```

- Assessment: Strong.

## 2. Orchestration — Missing

- Closest code today:
  - `src/load_coordinator.rs:23-65` — explicit `session/load` coordination, but only when a client reconnects and asks for resume.
  - `crates/fireline-conductor/src/runtime/mod.rs:23-221` and `src/runtime_host.rs:19-69` — runtime lifecycle and local auto-registration; this boots runtimes, it does not wake dormant work by session id.
  - `crates/fireline-control-plane/src/router.rs:22-163`, `crates/fireline-control-plane/src/main.rs:93-165`, `crates/fireline-control-plane/src/heartbeat.rs:6-40` — control-plane lifecycle, heartbeat tracking, and stale scanning.
  - `src/control_plane_client.rs:17-119` — retrying `register(...)` and a heartbeat loop; again lifecycle, not scheduler-owned wake.

- Relevant signatures:
```rust
// src/load_coordinator.rs:23-29
pub struct LoadCoordinatorComponent;
impl LoadCoordinatorComponent {
    pub fn new(session_index: crate::session_index::SessionIndex) -> Self;
}

// crates/fireline-conductor/src/runtime/mod.rs:23-221
pub struct RuntimeHost;
impl RuntimeHost {
    pub async fn create(&self, spec: CreateRuntimeSpec) -> Result<RuntimeDescriptor>;
    pub fn register(&self, runtime_key: &str, registration: RuntimeRegistration) -> Result<RuntimeDescriptor>;
    pub fn heartbeat(&self, runtime_key: &str, report: HeartbeatReport) -> Result<RuntimeDescriptor>;
    pub async fn stop(&self, runtime_key: &str) -> Result<RuntimeDescriptor>;
}

// src/control_plane_client.rs:17-119
pub struct ControlPlaneClient;
impl ControlPlaneClient {
    pub async fn register(&self, registration: RuntimeRegistration) -> Result<()>;
    pub fn spawn_heartbeat_loop(
        self: &Arc<Self>,
        metrics_source: impl Fn() -> HeartbeatMetrics + Send + Sync + 'static,
    ) -> JoinHandle<()>;
}
```

- Assessment: Missing.
- Closest analog: runtime lifecycle plus `session/load`; there is still no `wake(session_id)` or scheduler that can retry advancing suspended work by durable session identity.

## 3. Harness — Partial

- Closest code today:
  - `crates/fireline-conductor/src/build.rs:17-62` — composes the conductor, terminal, components, and trace writer.
  - `src/routes/acp.rs:34-81` — runtime-owned harness composition point per ACP WebSocket.
  - `crates/fireline-conductor/src/shared_terminal.rs:61-129,153-243` — long-lived subprocess plus attach/detach borrowing.
  - `crates/fireline-conductor/src/trace.rs:22-89` — appends harness progress to the durable session/state stream.

- Relevant signatures:
```rust
// crates/fireline-conductor/src/build.rs:18-44
pub fn build_subprocess_conductor(...) -> ConductorImpl<Agent>;
pub fn build_conductor_with_terminal(...) -> ConductorImpl<Agent>;

// crates/fireline-conductor/src/shared_terminal.rs:18-21,61-105
pub struct SharedTerminal;
impl SharedTerminal {
    pub async fn spawn(agent_command: Vec<String>) -> Result<Self>;
    pub async fn try_attach(&self) -> Result<SharedTerminalAttachment, AttachError>;
    pub async fn shutdown(&self) -> Result<()>;
}
```

- Assessment: Partial.
- Closest analog: Fireline owns an ACP proxy/conductor loop around the harness, but not a first-class `Effect<T> -> EffectResult<T>` interface independent of ACP traffic.

## 4. Sandbox — Strong

- Closest code today:
  - `crates/fireline-conductor/src/runtime/provider.rs:127-199` — `CreateRuntimeSpec`, `RuntimeProvider`, `RuntimeLaunch`, and `ManagedRuntime` define the provisioning contract.
  - `src/runtime_provider.rs:9-55` and `src/bootstrap.rs:103-223` — in-process local runtime provisioning.
  - `crates/fireline-control-plane/src/local_provider.rs:93-180,183-216` — control-plane subprocess provisioning and shutdown path.

- Relevant signatures:
```rust
// crates/fireline-conductor/src/runtime/provider.rs:127-199
pub struct CreateRuntimeSpec { pub agent_command: Vec<String>, pub state_stream: Option<String>, pub stream_storage: Option<StreamStorageConfig>, pub peer_directory_path: Option<PathBuf>, pub topology: TopologySpec, ... }
pub trait ManagedRuntime: Send {
    async fn shutdown(self: Box<Self>) -> Result<()>;
}
pub trait RuntimeProvider: Send + Sync {
    fn kind(&self) -> RuntimeProviderKind;
    async fn start(&self, spec: CreateRuntimeSpec, runtime_key: String, node_id: String) -> Result<RuntimeLaunch>;
}
```

- Assessment: Strong.

## 5. Resources — Missing

- Closest code today:
  - `crates/fireline-components/src/context.rs:45-52,169-200` — `WorkspaceFileSource` reads host-local files by direct path.
  - `crates/fireline-conductor/src/runtime/provider.rs:127-139` and `crates/fireline-control-plane/src/local_provider.rs:126-144` — raw local path fields such as `peer_directory_path` are threaded directly into launch.
  - `src/routes/files.rs:1-30` and `src/connections.rs:1-37` — planned helper-file and connection-lookup surfaces, still explicitly filesystem-local.

- Relevant signatures:
```rust
// crates/fireline-components/src/context.rs:50-52,176-183
pub trait ContextSource: Send + Sync {
    async fn gather(&self, session_id: &str) -> Result<String, sacp::Error>;
}
pub struct WorkspaceFileSource;
impl WorkspaceFileSource {
    pub fn new(path: impl Into<PathBuf>) -> Self;
}

// crates/fireline-conductor/src/runtime/provider.rs:129-138
pub struct CreateRuntimeSpec {
    pub peer_directory_path: Option<PathBuf>;
    pub topology: TopologySpec;
    ...
}
```

- Assessment: Missing.
- Closest analog: direct host-path plumbing and local file reads; I did not find a closer `[{source_ref, mount_path}]` abstraction hiding on `main`.

## 6. Tools — Strong

- Closest code today:
  - `crates/fireline-components/src/peer/mcp_server.rs:22-170` — typed MCP tools `list_peers` and `prompt_peer`.
  - `crates/fireline-components/src/smithery.rs:195-263` — typed `smithery_call` tool with explicit input/output schema.
  - `crates/fireline-components/src/lib.rs:1-41` — components crate treats MCP bridges as first-class runtime surfaces.

- Relevant signatures:
```rust
// crates/fireline-components/src/peer/mcp_server.rs:22-62
pub(crate) struct ListPeersInput {}
pub(crate) struct PromptPeerInput { pub agent_name: String, pub prompt: String }
pub(crate) struct PromptPeerOutput { pub runtime_id: String, pub agent_name: String, pub response_text: String, pub stop_reason: String }
pub(crate) fn build_peer_mcp_server(...) -> sacp::mcp_server::McpServer<Conductor, impl sacp::RunWithConnectionTo<Conductor>>;

// crates/fireline-components/src/smithery.rs:195-217
pub struct SmitheryCallInput { pub server: String, pub tool: String, pub arguments: serde_json::Value }
pub struct SmitheryCallOutput { pub result: serde_json::Value }
```

- Assessment: Strong.
