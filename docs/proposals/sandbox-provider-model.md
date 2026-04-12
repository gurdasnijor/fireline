# Unified SandboxProvider Model

> **Status:** architectural proposal.
> **Inspired by:** [Cased sandboxes](https://github.com/cased/sandboxes) — a Python ABC for multi-provider sandbox orchestration with auto-select, failover, and pooling.
> **Replaces:** the current three-layer dispatch stack (`SandboxProvider` trait + `SandboxDispatcher` + `fireline-host` router) plus the residual `RuntimeHost` / `RuntimeManager` vocabulary from the pre-primitive era.
> **Related:**
> - [`./deployment-and-remote-handoff.md`](./deployment-and-remote-handoff.md) — deployment topology that the provider model makes concrete
> - [`./fireline-host-audit.md`](./fireline-host-audit.md) — architecture issues this collapses
> - [`./cross-host-discovery.md`](./cross-host-discovery.md) — providers self-announce
> - [`./resource-discovery.md`](./resource-discovery.md) — providers mount discovered resources
> - [`./crate-restructure-manifest.md`](./crate-restructure-manifest.md) — the crate layout this proposal works within

---

## 1. TL;DR — one trait to rule them all

Every way Fireline can run an agent — local subprocess, microsandbox VM, Docker container, remote API call to another Fireline instance — is a `SandboxProvider` implementation. The "Host" is just an HTTP server that dispatches requests to whichever provider is configured. `bootstrap.rs` becomes provider-internal to the `LocalSubprocessProvider`, not a framework-level concern.

Today the codebase has three layers of indirection between an HTTP request and a running sandbox:

```
router.rs → SandboxDispatcher → SandboxProvider trait → LocalProvider / DockerProvider
                                                        ↕
                                              provider_trait.rs → LocalSandboxLauncher
                                                                   ↕
                                                          bootstrap.rs → SharedTerminal
```

Under this proposal that collapses to:

```
router.rs → ProviderDispatcher → SandboxProvider trait → LocalSubprocessProvider
                                                       → MicrosandboxProvider
                                                       → DockerProvider
                                                       → RemoteApiProvider
```

One dispatch layer. One trait. Four clean implementations. Everything else is internal to the provider that needs it.

---

## 2. The trait surface (Rust)

### SandboxProvider — the only trait that matters

```rust
#[async_trait]
pub trait SandboxProvider: Send + Sync {
    /// Human-readable provider name: "local", "microsandbox", "docker", "remote".
    fn name(&self) -> &str;

    /// What this provider can do. Dispatch uses this for auto-select and
    /// for rejecting requests that require capabilities the provider lacks.
    fn capabilities(&self) -> ProviderCapabilities;

    /// Provision a new sandbox from the given config. Returns a handle
    /// the caller uses for all subsequent operations.
    async fn create(&self, config: &SandboxConfig) -> Result<SandboxHandle>;

    /// Look up a sandbox by id. Returns None if the sandbox doesn't exist
    /// or belongs to a different provider.
    async fn get(&self, id: &str) -> Result<Option<SandboxDescriptor>>;

    /// List sandboxes, optionally filtered by labels.
    async fn list(&self, labels: Option<&HashMap<String, String>>) -> Result<Vec<SandboxDescriptor>>;

    /// Execute a command inside a running sandbox. Blocks until the
    /// command completes or the timeout fires.
    async fn execute(
        &self,
        id: &str,
        command: &str,
        timeout: Option<Duration>,
        env: Option<&HashMap<String, String>>,
    ) -> Result<ExecutionResult>;

    /// Destroy a sandbox. Idempotent: destroying a non-existent sandbox
    /// returns Ok(false).
    async fn destroy(&self, id: &str) -> Result<bool>;

    /// Provider-level health check. Returns true if the provider can
    /// accept create requests right now.
    async fn health_check(&self) -> Result<bool>;

    // --- optional convenience methods with defaults ---

    /// Find the first sandbox matching all given labels.
    async fn find(&self, labels: &HashMap<String, String>) -> Result<Option<SandboxDescriptor>> {
        Ok(self.list(Some(labels)).await?.into_iter().next())
    }

    /// Get an existing sandbox by labels, or create one if none matches.
    async fn get_or_create(
        &self,
        config: &SandboxConfig,
    ) -> Result<SandboxHandle> {
        if let Some(existing) = self.find(&config.labels).await? {
            Ok(SandboxHandle::from_descriptor(existing, self.name()))
        } else {
            self.create(config).await
        }
    }
}
```

### SandboxConfig — what the client sends

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxConfig {
    /// Human-readable name. Used for logging, labels, and default stream names.
    pub name: String,

    /// The agent binary + args to run inside the sandbox.
    pub agent_command: Vec<String>,

    /// Topology (combinator chain) the conductor should wire.
    #[serde(default)]
    pub topology: TopologySpec,

    /// Resources to mount inside the sandbox.
    #[serde(default)]
    pub resources: Vec<ResourceRef>,

    /// Durable-streams service URL. Required — no fallback to embedded.
    pub durable_streams_url: String,

    /// State stream name (auto-generated from sandbox id if omitted).
    pub state_stream: Option<String>,

    /// Environment variables visible to the agent process.
    #[serde(default)]
    pub env_vars: HashMap<String, String>,

    /// Labels for sandbox lookup, filtering, and pool reuse.
    #[serde(default)]
    pub labels: HashMap<String, String>,

    /// Provider hint. Auto-select if omitted.
    pub provider: Option<String>,
}
```

### SandboxHandle — what create() returns

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxHandle {
    pub id: String,
    pub provider: String,
    pub acp: Endpoint,
    pub state: Endpoint,
}
```

The handle carries the ACP and state-stream endpoints because **the caller needs them to talk to the sandbox directly** — there is no `sendInput` or `execute` wrapper at the Host primitive layer. ACP is a data-plane concern; the provider's job ends once the sandbox is reachable.

### SandboxDescriptor — full status + metadata

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxDescriptor {
    pub id: String,
    pub provider: String,
    pub status: SandboxStatus,
    pub acp: Endpoint,
    pub state: Endpoint,
    pub labels: HashMap<String, String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxStatus {
    Creating,
    Ready,
    Busy,
    Idle,
    Stopped,
    Broken,
}
```

### ProviderCapabilities

```rust
#[derive(Debug, Clone, Default)]
pub struct ProviderCapabilities {
    /// Can stream stdout/stderr during execute().
    pub streaming: bool,
    /// Can upload/download files to/from the sandbox.
    pub file_transfer: bool,
    /// Can mount OCI images as rootfs.
    pub oci_images: bool,
    /// Can mount durable-stream-blob resources.
    pub stream_resources: bool,
    /// Can snapshot and restore sandbox state.
    pub snapshots: bool,
    /// Supports GPU passthrough.
    pub gpu: bool,
    /// Supports hardware-level VM isolation.
    pub vm_isolation: bool,
}
```

### ExecutionResult

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub timed_out: bool,
}
```

---

## 3. Concrete providers

### LocalSubprocessProvider

**Absorbs:** `bootstrap.rs`, `ChildProcessSandboxLauncher` (`local_provider.rs`), `SharedTerminal`, and the current direct-host boot path.

```
create(config) → {
  1. Prepare resources via ResourceMounter
  2. Spawn `fireline` binary as child process with --host/--port/--durable-streams-url/etc.
  3. Wait for the child to register Ready (via stream projection, not RuntimeRegistry)
  4. Return SandboxHandle { id: host_key, acp: ws://..., state: http://... }
}

execute(id, cmd) → {
  Shell into the child process (or exec via ACP if supported)
}

destroy(id) → {
  SIGINT → wait → SIGKILL; emit stopped event to stream
}
```

**Capabilities:** `{ file_transfer: true, stream_resources: true }`.

### MicrosandboxProvider

**Absorbs:** `crates/fireline-sandbox/src/microsandbox.rs` + the microsandbox spike at `/tmp/fireline-microsandbox-spike/`.

```
create(config) → {
  1. Sandbox::builder(id).image(config.image).cpus/memory
       .network(|n| n.policy(allow_all))
       .volume(...)
       .env_vars(...)
       .create_detached()
  2. sandbox.fs().copy_from_host(fireline_bin, "/usr/local/bin/fireline")
  3. sandbox.shell("fireline --host 0.0.0.0 --port 4437 ... -- <agent> &")
  4. Wait for Ready via stream projection
  5. Return SandboxHandle { id, acp, state }
}

execute(id, cmd) → sandbox.exec(cmd)

destroy(id) → sandbox.stop_and_wait() + remove_persisted()
```

**Capabilities:** `{ vm_isolation: true, oci_images: true, file_transfer: true, stream_resources: true }`.

### DockerProvider

**Absorbs:** `crates/fireline-sandbox/src/providers/docker.rs`.

```
create(config) → {
  1. docker.build_image() (lazy, cached)
  2. docker.create_container() + start_container()
  3. Wait for Ready
  4. Return SandboxHandle
}

execute(id, cmd) → docker.exec_create + exec_start

destroy(id) → docker.stop_container + remove_container(force)
```

**Capabilities:** `{ oci_images: true, file_transfer: true, stream_resources: true }`.

### RemoteApiProvider

**New.** Wraps HTTP calls to another Fireline host's `POST /v1/sandboxes` endpoint. This is how cross-host provisioning works: the local provider dispatch discovers a remote host on the `hosts:tenant-<id>` stream, and provisions a sandbox there via the remote host's API.

```
create(config) → POST {remote_url}/v1/sandboxes { config }
get(id)        → GET  {remote_url}/v1/sandboxes/{id}
destroy(id)    → DELETE {remote_url}/v1/sandboxes/{id}
execute(id, cmd) → POST {remote_url}/v1/sandboxes/{id}/exec { command, timeout }
```

**Capabilities:** depends on the remote host's provider; discovered via `GET {remote_url}/v1/capabilities`.

---

## 4. Provider dispatch + auto-selection

### ProviderDispatcher

Replaces `SandboxDispatcher`, `RuntimeManager`, and `RuntimeHost`. One struct, one responsibility: route requests to the right provider.

```rust
pub struct ProviderDispatcher {
    providers: Vec<Arc<dyn SandboxProvider>>,
    default_provider: usize,  // index into providers
    read_model: Arc<HostIndex>,
}

impl ProviderDispatcher {
    pub fn new(primary: Arc<dyn SandboxProvider>, read_model: Arc<HostIndex>) -> Self { ... }
    pub fn with_fallback(mut self, fallback: Arc<dyn SandboxProvider>) -> Self { ... }

    pub async fn provision(&self, config: SandboxConfig) -> Result<SandboxHandle> {
        let provider = self.select_provider(&config)?;
        provider.create(&config).await
    }

    fn select_provider(&self, config: &SandboxConfig) -> Result<&Arc<dyn SandboxProvider>> {
        // 1. If config.provider is Some, find by name
        // 2. Otherwise use the default
        // 3. Verify provider.capabilities() satisfies config requirements
    }

    pub async fn provision_with_failover(&self, config: SandboxConfig) -> Result<SandboxHandle> {
        for provider in &self.providers {
            match provider.health_check().await {
                Ok(true) => match provider.create(&config).await {
                    Ok(handle) => return Ok(handle),
                    Err(e) => tracing::warn!(provider = provider.name(), error = %e, "provision failed, trying next"),
                },
                _ => continue,
            }
        }
        Err(anyhow!("all providers failed or unhealthy"))
    }
}
```

### How this replaces the current stack

| Current | After |
|---|---|
| `SandboxDispatcher::provision()` | `ProviderDispatcher::provision()` |
| `SandboxDispatcher::resolve()` → `SandboxProvider::provision()` | `ProviderDispatcher::select_provider()` → `SandboxProvider::create()` |
| `SandboxDispatcher::stop()` → manual `shutdown()` + emit | `SandboxProvider::destroy()` (each provider owns its cleanup + emit) |
| `SandboxDispatcher::ensure_read_model_stream()` | Read model wired at `ProviderDispatcher` construction, not lazily |
| `SandboxDispatcher::active_launches` (in-memory handle map) | Each provider owns its handles internally; the dispatcher is stateless |
| `RuntimeManager` (defunct) | Deleted |
| `RuntimeHost` (defunct) | Deleted |

---

## 5. Pooling (future)

Cased's `SandboxPool` wraps any `SandboxProvider` and pre-warms N sandboxes. We adopt the same pattern:

```rust
pub struct PooledProvider {
    inner: Arc<dyn SandboxProvider>,
    pool: Arc<Mutex<SandboxPool>>,
    config: PoolConfig,
}

pub struct PoolConfig {
    pub min_idle: usize,       // Pre-warm target
    pub max_total: usize,      // Hard cap
    pub max_idle: usize,       // Evict excess idle
    pub sandbox_ttl: Duration, // Max sandbox lifetime
    pub idle_timeout: Duration,// Evict after idle period
    pub strategy: PoolStrategy,
}

pub enum PoolStrategy { Lazy, Eager, Hybrid }
```

`PooledProvider` implements `SandboxProvider` — it wraps `create()` with pool-acquire logic and `destroy()` with pool-release. The pool background task pre-warms on `Eager`, evicts on TTL, and LRU-evicts idle sandboxes when the pool exceeds `max_idle`.

**When to pool:**
- High-traffic APIs where sandbox boot time dominates latency → `Eager` with `min_idle: 3`.
- Dev / one-off runs → `Lazy` (default). No pre-warming; pool is effectively a handle cache.
- Mixed → `Hybrid`: pre-warm 1, scale to `max_total` on demand.

**Label-based reuse:** if a request's labels match an idle pooled sandbox, reuse it instead of creating a new one. This is Cased's `reuse_by_labels` pattern — check the pool for a label-match before falling through to `create()`.

Not on the critical path. The trait surface is designed to support pooling without breaking changes — `PooledProvider` is just another `SandboxProvider` satisfier.

---

## 6. What collapses

### fireline-host crate → thin HTTP server + ProviderDispatcher

**Before (today):**
```
fireline-host/src/
├── auth.rs                    ← deleted in C3
├── bootstrap.rs               ← moves INSIDE LocalSubprocessProvider
├── control_plane.rs           ← HostServerConfig + dispatcher wiring
├── control_plane_client.rs    ← moves inside LocalSubprocessProvider (heartbeat is provider-internal)
├── control_plane_peer_registry.rs ← deleted in C4
├── heartbeat.rs               ← deleted in C5
├── lib.rs
├── local_provider.rs          ← absorbed into LocalSubprocessProvider
├── router.rs                  ← simplified to dispatch to ProviderDispatcher
└── runtime_host.rs            ← deleted in C2
```

**After:**
```
fireline-host/src/
├── lib.rs
├── router.rs      ← ~150 lines: POST/GET/DELETE /v1/sandboxes + healthz
└── server.rs      ← ~100 lines: bind, configure ProviderDispatcher, serve
```

~250 lines total. Everything else moved into `fireline-sandbox` (the provider impls) or deleted.

### fireline-sandbox crate → owns the trait + all providers

**Before:** trait (`provider.rs`) + dispatcher (`dispatcher.rs`) + provider-trait (`provider_trait.rs`) + registry + stream_trace + providers/ + microsandbox.

**After:**
```
fireline-sandbox/src/
├── lib.rs                     ← re-exports
├── trait.rs                   ← SandboxProvider + SandboxConfig + SandboxHandle + etc.
├── dispatcher.rs              ← ProviderDispatcher (simplified from SandboxDispatcher)
├── stream_trace.rs            ← emit_host_spec/endpoints_persisted (unchanged)
├── providers/
│   ├── local.rs               ← LocalSubprocessProvider (absorbs bootstrap.rs + local_provider.rs)
│   ├── docker.rs              ← DockerProvider (absorbs providers/docker.rs)
│   ├── microsandbox.rs        ← MicrosandboxProvider (absorbs microsandbox.rs)
│   └── remote.rs              ← RemoteApiProvider (NEW)
├── pool.rs                    ← PooledProvider (future)
└── registry.rs                ← RuntimeRegistry (pending deletion in C5)
```

### What specific types collapse

| Current type | New type | Notes |
|---|---|---|
| `SandboxProvider` (trait, 3 methods) | `SandboxProvider` (trait, 8+ methods) | Expanded to match Cased's ABC |
| `SandboxDispatcher` | `ProviderDispatcher` | Simplified — no `ensure_read_model_stream`, no `active_launches` |
| `LocalSandboxLauncher` (trait) | absorbed into `LocalSubprocessProvider` | The indirection through a launcher trait is gone |
| `ChildProcessSandboxLauncher` | `LocalSubprocessProvider` | Direct implementation |
| `BootstrapRuntimeLauncher` | deleted | Direct-host mode calls the provider directly |
| `ManagedSandbox` (trait) | absorbed into each provider's `destroy()` | Each provider owns its cleanup |
| `SandboxLaunch` | `SandboxHandle` | Simpler — no `Box<dyn ManagedSandbox>` field |
| `ProvisionSpec` | `SandboxConfig` | Renamed and simplified (no `host` / `port` — those are provider-internal) |
| `HostDescriptor` | `SandboxDescriptor` | Renamed |
| `HostStatus` | `SandboxStatus` | Renamed |
| `SandboxProviderKind` (enum) | `provider.name()` (string) | String-based, not enum-based — extensible without code changes |
| `SandboxProviderRequest` (enum: Auto/Local/Docker) | `config.provider: Option<String>` | String-based selection |
| `SandboxTokenIssuer` | deleted | Auth is a server concern, not a provider concern |
| `RuntimeRegistry` | deleted (C5) | Stream projection replaces it |

### AppState → simplified

```rust
pub struct AppState {
    pub dispatcher: ProviderDispatcher,
    pub config: HostServerConfig,
}

pub struct HostServerConfig {
    pub bind_addr: SocketAddr,
    pub durable_streams_url: String,
    pub discovery_stream: Option<String>,  // hosts:tenant-<id> for self-announcement
}
```

---

## 7. Cutover plan

### Phase P1 — Expand the `SandboxProvider` trait to the target surface

**Scope:** Add `name()`, `capabilities()`, `get()`, `list()`, `execute()`, `health_check()`, `find()`, `get_or_create()` to the trait. Existing `provision()` becomes `create()`. Default implementations for optional methods.

**Files:** `crates/fireline-sandbox/src/provider.rs` (rewrite trait), `providers/local.rs`, `providers/docker.rs` (add stub impls for new methods).

**Risk:** Low — additive. Existing callers use `create()` (renamed from `provision()`).

**Commits:** 2. (1: expand trait + update local impl. 2: update docker impl.)

### Phase P2 — Introduce `ProviderDispatcher` alongside `SandboxDispatcher`

**Scope:** New `ProviderDispatcher` struct implementing the auto-select + failover logic. `SandboxDispatcher` stays as a backward-compat alias during the transition.

**Files:** `crates/fireline-sandbox/src/dispatcher.rs` (rewrite), `crates/fireline-host/src/router.rs` (switch from `SandboxDispatcher` to `ProviderDispatcher`).

**Risk:** Medium — router.rs is a central file.

**Commits:** 2. (1: new dispatcher. 2: router switch.)

### Phase P3 — Absorb `bootstrap.rs` + `local_provider.rs` into `LocalSubprocessProvider`

**Scope:** Move `bootstrap::start()` and the `ChildProcessSandboxLauncher` into `providers/local.rs` as an internal `create()` implementation. Delete the indirection through `LocalSandboxLauncher` trait.

**Files:** `crates/fireline-sandbox/src/providers/local.rs` (absorb), `crates/fireline-host/src/bootstrap.rs` (delete or thin to a re-export), `crates/fireline-host/src/local_provider.rs` (delete), `crates/fireline-sandbox/src/provider_trait.rs` (delete).

**Risk:** High — the local subprocess launch path is the most exercised path in the codebase.

**Commits:** 3-4. (1: move bootstrap into local provider. 2: move child-process launcher. 3: delete old files. 4: fix tests.)

### Phase P4 — Add `RemoteApiProvider` + hook into cross-host discovery

**Scope:** New provider that POSTs to another Fireline host's API. Wire it into `ProviderDispatcher` as a failover option. Integrate with the `hosts:tenant-<id>` discovery stream for remote host lookup.

**Files:** `crates/fireline-sandbox/src/providers/remote.rs` (new), `crates/fireline-sandbox/src/dispatcher.rs` (add remote provider registration).

**Risk:** Medium — new code, no existing callers to break.

**Commits:** 2. (1: provider impl. 2: dispatcher + discovery integration.)

### Phase P5 — Thin `fireline-host` to ~250 lines

**Scope:** Delete everything from `fireline-host` that has been absorbed by providers. What remains: `router.rs` (dispatch to `ProviderDispatcher`), `server.rs` (bind + serve), `lib.rs` (re-exports).

**Files:** Delete `bootstrap.rs`, `control_plane.rs`, `control_plane_client.rs`, `heartbeat.rs`, `auth.rs`, `local_provider.rs`, `control_plane_peer_registry.rs`, `runtime_host.rs`, `provider_trait.rs` (or whatever survives the cleanup plan). Keep `router.rs`, `server.rs`, `lib.rs`.

**Risk:** High — biggest deletion. All integration tests must pass against the provider model.

**Commits:** 2-3. (1: delete absorbed files. 2: simplify router. 3: fix tests.)

### Phase P6 — Wire rename on the HTTP surface

**Scope:** Rename the HTTP endpoints from `/v1/runtimes` to `/v1/sandboxes`. The old paths stay as aliases for one release cycle. Update TS client to use new paths.

**Risk:** Medium — wire-format change. Needs a deprecation period.

**Commits:** 2. (1: add new paths + aliases. 2: update TS client.)

**Total: ~15-19 commits across 6 phases.** Phases P1-P2 are safe to land immediately. P3 is the critical path. P4-P6 are incremental after P3.

---

## 8. How this composes with other proposals

### Cross-host discovery ([`./cross-host-discovery.md`](./cross-host-discovery.md))

Providers self-announce to the `hosts:tenant-<id>` stream at `create()` time. The `RemoteApiProvider` reads that stream to discover remote hosts. The current `ControlPlanePeerRegistry` (an HTTP-poll adapter) is replaced by a stream-backed `StreamDeploymentPeerRegistry` in `fireline-tools` that reads from the same discovery stream. The provider model makes the self-announcement mechanical: every provider emits `host_registered` at startup and `host_deregistered` at shutdown.

### Resource discovery ([`./resource-discovery.md`](./resource-discovery.md))

Providers mount discovered resources at `create()` time. The `SandboxConfig.resources` field carries `ResourceRef` values that may include `DurableStreamBlob` refs. Each provider's `create()` implementation calls `ResourceMounter::mount()` — `LocalSubprocessProvider` uses `LocalPathMounter` for local paths and `DurableStreamMounter` for stream-backed blobs; `MicrosandboxProvider` uses `.volume()` for local paths and `fs().copy_from_host()` for stream-fetched blobs; etc.

### SecretsInjection ([`./deployment-and-remote-handoff.md`](./deployment-and-remote-handoff.md) §5)

Providers inject credentials at `create()` time via `SandboxConfig.env_vars`. The `SecretsInjectionComponent` (a Harness combinator, not a provider concern) strips real secrets from the agent's visible environment and replaces them with placeholders; the TLS proxy in the sandbox resolves placeholders to real values only for allowed hosts. The provider's job is to ensure the env_vars reach the agent process — how they're protected is a Harness concern. Microsandbox's `.secret_env()` API maps directly.

### Stream-FS ([`./stream-fs-spike.md`](./stream-fs-spike.md))

Providers mount stream-FS snapshots as a `ResourceRef::StreamFs` variant. The `DurableStreamMounter` resolves the snapshot from the `resources:tenant-<id>` stream and materializes it to a tmpfs before bind-mounting. From the provider's perspective, this is just another resource mount — the `ResourceMounter` abstraction handles the stream-FS specifics.

---

## 9. Comparison to Cased

### What we adopt from their model

| Cased pattern | Fireline equivalent |
|---|---|
| `SandboxProvider` ABC with `create/get/list/execute/destroy` | `SandboxProvider` trait with the same five methods + `name/capabilities/health_check` |
| `ProviderCapabilities` as feature flags | Same shape — `streaming`, `file_transfer`, `oci_images`, `vm_isolation`, etc. |
| `SandboxManager` with auto-select + failover | `ProviderDispatcher` with `select_provider()` + `provision_with_failover()` |
| `SandboxPool` wrapping any provider with pre-warming + LRU eviction | `PooledProvider` wrapping any `SandboxProvider` (future) |
| `SandboxConfig` carrying name, image, env, labels, resources | Same — `SandboxConfig` with `name`, `agent_command`, `topology`, `resources`, `env_vars`, `labels` |
| `Sandbox` handle with id + provider + state | `SandboxHandle` with id + provider + `acp` + `state` endpoints |
| `ExecutionResult` with exit_code + stdout + stderr + duration + timed_out | Identical shape |
| Label-based lookup and reuse (`find`, `get_or_create`) | Same — `find(labels)` and `get_or_create(config)` with default impls |

### What differs

| Dimension | Cased | Fireline |
|---|---|---|
| **Data plane** | Provider owns stdin/stdout; `execute()` is the primary interaction surface | Provider hands back `acp` + `state` endpoints; the caller speaks ACP directly. `execute()` is a secondary convenience, not the main path. |
| **State substrate** | No durable state; each sandbox is ephemeral | Durable-streams substrate: every sandbox writes to a state stream, and the read model is a projection over that stream. |
| **Discovery** | None — the manager is a local registry | Providers self-announce to a `hosts:tenant-<id>` durable stream. Cross-host discovery is automatic. |
| **Formal verification** | None | `verification/spec/deployment_discovery.tla` checks `HostRegisteredIsEventuallyDiscoverable`, `StaleHeartbeatCollapsesToInvisible`, `RuntimeDependentOnHost`, etc. |
| **Provider identification** | Enum-based (`'e2b'`, `'daytona'`, `'modal'`) | String-based (`provider.name()`) — extensible without changing the trait |
| **Topology / agent composition** | Not modeled — the sandbox runs whatever image you give it | `SandboxConfig.topology` carries a `TopologySpec` (combinator chain) that the conductor inside the sandbox interprets |
| **Resource mounting** | `upload_file()` / `download_file()` on the provider | `SandboxConfig.resources` carries `ResourceRef` values; the provider's `create()` calls `ResourceMounter::mount()` which handles local paths, stream blobs, OCI layers, git repos, etc. |
| **Pooling lifecycle** | Pool owns the provider reference; acquire/release pattern | Same pattern, but the pool is itself a `SandboxProvider` impl (`PooledProvider`), so it composes transparently with the dispatcher |

---

## 10. What this does NOT change

- **The Anthropic primitive taxonomy** (Session, Orchestration, Harness, Sandbox, Resources, Tools) — the six primitives stay. `SandboxProvider` is how the Sandbox primitive gets satisfied; the taxonomy doesn't change.
- **The durable-streams substrate** — every sandbox still writes to a durable stream via `DurableStreamTracer`. The `SandboxConfig` requires a `durable_streams_url` (no fallback to embedded). Stream-as-truth is reinforced, not weakened.
- **The TLA verification layer** — `managed_agents.tla` and `deployment_discovery.tla` stay. The provider model is the concrete implementation of invariants those specs already check.
- **The TS client surface** — `@fireline/client/host` still exports `Host.provision / wake / status / stop`. The `createFirelineHost` satisfier wraps the HTTP API, which now routes to `ProviderDispatcher`. The client doesn't know or care about the internal dispatch change.
- **The browser harness** — still calls `host.provision(...)` via the vite proxy. The proxy target is the same HTTP server; only the internal dispatch path changes.
- **The managed-agent test suite** — `tests/managed_agent_*.rs` test the substrate end-to-end through the HTTP API. The API shape doesn't change (until the optional P6 rename); only the internal plumbing does. Every test should pass unmodified through P1-P5.

---

## Appendix: current → target type mapping

| Current (crates/fireline-sandbox/) | Target | Cased equivalent |
|---|---|---|
| `SandboxProvider` trait (3 methods) | `SandboxProvider` trait (8+ methods) | `SandboxProvider` ABC |
| `SandboxDispatcher` | `ProviderDispatcher` | `SandboxManager` |
| `SandboxLaunch` | `SandboxHandle` (no `Box<dyn ManagedSandbox>`) | `Sandbox` handle |
| `ManagedSandbox` trait | deleted (each provider owns cleanup) | (not modeled) |
| `LocalSandboxLauncher` trait | deleted (absorbed into `LocalSubprocessProvider`) | (not modeled) |
| `ProvisionSpec` | `SandboxConfig` | `SandboxConfig` |
| `HostDescriptor` | `SandboxDescriptor` | `BaseSandbox` |
| `HostStatus` | `SandboxStatus` | `SandboxState` |
| `SandboxProviderKind` (enum) | `provider.name()` (string) | `provider.name` (property) |
| `SandboxProviderRequest` (enum) | `config.provider: Option<String>` | explicit provider param |
| `RuntimeRegistry` | deleted (stream projection) | (not modeled — Cased has no durable state) |
| (none) | `ProviderCapabilities` | `ProviderCapabilities` |
| (none) | `ExecutionResult` | `ExecutionResult` |
| (none) | `PooledProvider` (future) | `SandboxPool` |
| (none) | `RemoteApiProvider` | (no equivalent — Cased doesn't do cross-host) |
