# Hosting Primitives — Critical Architectural Review

> Date: 2026-04-12
> Reviewer: Architect (Opus 3)
> Scope: `crates/fireline-host/src/bootstrap.rs`, `control_plane.rs`, `router.rs`, and the host-adjacent surface in `crates/fireline-harness/src/{host_topology,routes_acp,trace}.rs`.
> Purpose: identify architectural smells in the hosting primitives and propose a concrete refactor path. Not a veto of anything shipping.

## TL;DR

The hosting layer works but carries real debt. Three headline issues:

1. **Two boot paths with almost no shared substrate.** `bootstrap.rs` (direct-host, 396 lines) and `control_plane.rs` (provisioning host, 101 lines) each recreate listener/router/producer/materializer wiring. A missing `HostCore` primitive would collapse both into thin compositions.
2. **Layering inversion: `fireline-host` depends on `fireline-harness`.** Host-specific types (`ComponentContext`, `AcpRouteState`, `emit_host_*`, `build_host_topology_registry`) live in the harness crate. Harness is meant to be the *internal session-execution substrate*; owning host-orchestration concepts forces the dependency graph upside-down.
3. **Load-bearing ordering comments + redundant event vocabulary.** Boot emits 5 events across 2 streams with a hard-coded ordering imposed by a materializer empty-stream-exit bug. Shutdown emits 3 more events, each wrapped in `.context(...)` with no rollback if an early step fails.

Nothing here is demo-blocking. Recommending a dedicated refactor phase after the canonical-ids cascade finishes; `HostCore` extraction is the first move.

---

## 1. Observations — what the layer does today

### `bootstrap.rs` (direct-host boot, 396 lines)

Responsibilities (roughly in `start()` order):
- Generate host UUID, build host_id, state_stream name, host+stream URLs.
- Bind TCP listener; derive the "connectable" advertised URL via `connect_host()`.
- Build two `durable-streams` Producers (state + deployment discovery).
- Construct `SessionIndex`, `StateMaterializer`, `SharedTerminal` (the agent subprocess).
- Ensure named streams exist; ensure state + deployment streams exist.
- Construct `StreamDeploymentPeerRegistry`.
- Call `build_host_topology_registry(ComponentContext {...})` — 8-field context threaded across crates.
- Build `AcpRouteState` with a `base_components_factory` closure producing `LoadCoordinatorComponent`.
- Compose axum router: `/healthz` + `fireline_harness::routes_acp::router(app_state)`.
- Spawn server task with graceful shutdown.
- Emit a sequence of events on two streams:
  - state stream: `host_spec_persisted`, `host_instance_started`
  - deployment stream: `HostRegistered`, `HostProvisioned`, then heartbeat loop of `HostHeartbeat`
- Preload the materializer.
- Return a `BootstrapHandle` holding 7+ long-lived resources.

### `control_plane.rs` (provisioning host boot, 101 lines)

- Bind listener, construct `HostIndex`, construct `ProviderDispatcher` from `LocalSubprocessProvider` / `DockerProvider` / (optional) `RemoteAnthropicProvider`.
- Build router (from `router.rs`) with `AppState { dispatcher, infra }`.
- Serve.

### `router.rs` (sandbox CRUD)

- `POST /v1/sandboxes`, `GET /v1/sandboxes`, `GET/DELETE /v1/sandboxes/{id}`, `POST /v1/sandboxes/{id}/stop`.
- `ControlPlaneError` converts any anyhow to 500 — loses all status signal from providers.

### `fireline-harness/src/host_topology.rs` (+ `routes_acp.rs`, `trace.rs`)

- Defines `ComponentContext`, `AcpRouteState`, `build_host_topology_registry`, `emit_host_*` helpers — **host-layer concepts living in the harness crate**.

---

## 2. Smells — ranked by architectural cost

### S1. Two boot paths, one missing primitive **(HIGH)**

`bootstrap.rs` and `control_plane.rs` do not share a `HostCore` abstraction. Each independently:

- binds a listener
- builds a router
- emits host-lifecycle events (direct-host does; control-plane doesn't — asymmetric)
- manages a state/deployment producer pair (direct-host does; control-plane only has the provisioning API)

When the next target comes along (Tier C spec-stream host, OCI quickstart, managed-runtime-only host), there's no shared substrate to compose. You'll write a third 300-line `bootstrap` variant.

### S2. Layering inversion **(HIGH)**

Dependency direction:
```
fireline-host  →  fireline-harness  →  fireline-session
             ↘                    ↘
               fireline-orchestration, fireline-resources, fireline-tools
```

But conceptually:
```
fireline-host (outer process)  →  fireline-harness (session execution)  →  fireline-session (state types)
```

The harness *shouldn't* know about host lifecycle. Today it does:
- `ComponentContext` carries `host_key`, `host_id`, `node_id` — host-plane identity
- `emit_host_spec_persisted`, `emit_host_instance_started/stopped`, `emit_host_endpoints_persisted` all live in `fireline-harness/src/trace.rs`
- `AcpRouteState` holds `conductor_name`, `host_key`, `host_id`, `node_id` — host identity in a harness surface

This is the reason `bootstrap.rs` is 396 lines — it's forced to import cross-crate host helpers and thread them through a harness-owned `ComponentContext`.

**Rule of thumb:** if moving a struct *out* of crate X requires pulling crate X's dependencies with it, the struct is in the wrong crate.

### S3. Load-bearing ordering comments **(MEDIUM)**

Lines 226-230:

```rust
// Emit stream events BEFORE the materializer preloads, so
// the stream has content when the materializer subscribes.
// Without this ordering, preload connects to an empty stream,
// the worker finds nothing to replay, and exits — causing
// "state materializer worker exited before preload completed."
```

This is a bug workaround masquerading as architecture. The fix belongs in `StateMaterializer` (or the durable-streams reader):
- Materializer's empty-stream-exit behavior is wrong. Either: (a) preload should block until first envelope with a timeout, (b) materializer should tolerate empty log and subscribe live, (c) materializer should expose a `live_from_empty()` seed option.

Until fixed, every future boot path has to replicate the same "seed before preload" dance.

### S4. Redundant event vocabulary **(MEDIUM)**

Boot-time events, across two streams:

| Stream | Event | Semantic |
|---|---|---|
| state stream | `host_spec_persisted` | "this host's config is durable" |
| state stream | `host_instance_started` | "process instance is live" |
| deployment stream | `HostRegistered` | "host exists in discovery" |
| deployment stream | `HostProvisioned` | "host is ready to serve" |
| deployment stream | `HostHeartbeat` | "host is still alive" (periodic) |

`HostRegistered` and `HostProvisioned` are emitted back-to-back with the same timestamp. What distinguishes them? If nothing, consolidate. If something (e.g., capabilities take a moment to come up), make the timestamp difference meaningful.

Shutdown emits `HostStopped`, `HostDeregistered`, `host_instance_stopped` — three events for one transition. Ditto redundant.

**Proposed vocabulary (one event set across both streams):**
- `host.present` (emitted once on successful boot, carries full descriptor including capabilities)
- `host.heartbeat` (periodic)
- `host.gone` (emitted on graceful shutdown OR after stale-heartbeat threshold by a janitor subscriber)

Readers project these into whatever internal shape they need (SessionIndex, HostIndex, peer registry).

### S5. Shutdown sequence has no rollback **(MEDIUM)**

`BootstrapHandle::shutdown()` does:

```
abort heartbeat → await heartbeat → emit HostStopped → emit HostDeregistered →
emit host_instance_stopped → abort materializer → send shutdown_tx →
shutdown shared_terminal → await server_task
```

Each `emit` uses `.context(...).?`. If `emit HostStopped` fails (network glitch), the remaining steps are skipped:
- materializer not aborted → leaks a task
- shutdown_tx not sent → server keeps running until process dies
- shared_terminal not shut down → agent subprocess lives on

**No structured rollback.** For a long-lived host, this is tolerable (OS reclaims on exit). For embedded uses (tests spawning + shutting down hosts repeatedly), this leaks.

**Fix:** collect best-effort errors from each step, always proceed to the next, aggregate and return. Pattern:

```rust
async fn shutdown(mut self) -> Result<()> {
    let mut errors = Vec::new();
    if let Err(e) = self.emit_host_stopped().await { errors.push(e); }
    if let Err(e) = self.emit_host_deregistered().await { errors.push(e); }
    // ...always abort every task, always send shutdown_tx, always drain server_task
    errors.into_iter().next().map_or(Ok(()), Err)
}
```

### S6. Dead / misleading fields **(LOW)**

`BootstrapConfig.control_plane_url: Option<String>` is never read in `bootstrap.rs`. Dead. Remove.

`PersistedHostSpec` constructed at line 231 has `provider: SandboxProviderRequest::Local` hardcoded — even though direct-host doesn't use the provider abstraction at all. Misleading.

`stream_storage: None`, `peer_directory_path: None` — defaults inline. Either give `PersistedHostSpec` a builder with safer defaults, or extract the spec construction into a helper.

### S7. `agent_command_for_spec` clone workaround **(LOW)**

Lines 171-174:

```rust
// Keep a clone of the agent command around so we can thread it into
// the `host_spec` envelope further down — SharedTerminal::spawn
// consumes the original.
let agent_command_for_spec = config.agent_command.clone();
let shared_terminal = SharedTerminal::spawn(config.agent_command).await?;
```

The root issue: `SharedTerminal::spawn` takes `Vec<String>` by value. If it took `&[String]` or `Arc<[String]>`, no clone. Small issue individually, symptomatic of "owned vs borrowed" friction across this layer.

### S8. `connect_host()` URL generation is wrong for containers **(LOW)**

```rust
fn connect_host(ip: IpAddr) -> String {
    if ip.is_unspecified() {
        match ip {
            IpAddr::V4(_) => "127.0.0.1".to_string(),
            ...
        }
    } else { ip.to_string() }
}
```

When bound to `0.0.0.0`, the advertised `acp_url = "ws://127.0.0.1:{port}/acp"` is unreachable from outside the container. For local-dev this is fine. For any hosted deployment, the bootstrap has NO way to know its externally-reachable URL. Needs:
- optional `FIRELINE_ADVERTISED_ACP_URL` env override (already exists in `control_plane.rs` via `--advertised-acp-url` flag)
- OR defer URL emission until a peer/reverse-proxy layer supplies it

### S9. `base_components_factory: Arc<dyn Fn() -> Vec<...>>` **(LOW)**

Currently produces exactly one thing: `LoadCoordinatorComponent`. Factory indirection for one value is YAGNI. Replace with a direct `Vec<DynConnectTo>` or a typed struct. If a second factory consumer appears, re-abstract then.

### S10. `BootstrapHandle` public field duplication **(LOW)**

Public `host_id: String`, `host_key: String`, `host_created_at: i64` on the handle. These are also stored internally for shutdown's event emission. If anyone mutates the public field, shutdown logs use a stale value. Either make them `Arc<String>` shared, or make them getter methods that read the canonical internal copy.

### S11. Direct-host boot emits a `PersistedHostSpec` with infra fields **(LOW)**

Per canonical-ids Phase 7, infra fields live in the infra plane (`hosts:tenant-{id}`). Direct-host's `persisted_spec` contains `host_key`, `node_id`, `provider: Local`, `host` (IP), `port` — correctly infra-plane. It emits to `state_stream_url` (agent plane) though. Line 250:

```rust
emit_host_spec_persisted(&state_stream_url, &persisted_spec)
```

This is arguably a plane-separation violation — host identity lands on the *agent-plane state stream* instead of the hosts stream. Verify this is intentional (maybe the direct-host test harness needs it there). If not, move to `host_stream_url`.

---

## 3. Recommended refactor

### R1. Extract `fireline-host-core`

New module (or crate) containing the shared substrate:

```rust
pub struct HostCore {
    listener: TcpListener,
    state_producer: Producer,
    deployment_producer: Producer,
    state_materializer_task: StateMaterializerTask,
    shared_heartbeat: HeartbeatTask,
}

pub struct HostCoreConfig {
    pub host: IpAddr,
    pub port: u16,
    pub name: String,
    pub durable_streams_url: String,
    pub advertised_base_url: Option<String>,  // replaces connect_host hack
    pub state_stream: Option<String>,
    pub identity: HostIdentity,  // host_key, node_id, provider_instance_id
}

impl HostCore {
    pub async fn start(config: HostCoreConfig) -> Result<Self>;
    pub fn acp_url(&self) -> &str;
    pub fn state_stream_url(&self) -> &str;
    pub fn state_producer(&self) -> &Producer;
    pub fn compose_router(self, app: Router) -> ServingHost;
    pub async fn shutdown(self) -> Result<()>;  // best-effort; collects errors
}
```

Both `bootstrap.rs` and `control_plane.rs` become thin compositions:

- `bootstrap::start()` = HostCore + SharedTerminal + LoadCoordinator base component + harness routes_acp router.
- `control_plane::run_host()` = HostCore + ProviderDispatcher + sandboxes router.

Est LOC saved: `bootstrap.rs` 396 → ~120. `control_plane.rs` ~unchanged (it's already thin). Gains: one place to fix materializer ordering, one event vocabulary, one shutdown sequence.

### R2. Flip the layering

Move from `fireline-harness` to `fireline-host-core`:
- `ComponentContext`
- `build_host_topology_registry`
- `emit_host_spec_persisted`, `emit_host_instance_started/stopped`, `emit_host_endpoints_persisted`
- Host-identity fields in `AcpRouteState` (conductor_name, host_key, host_id, node_id)

`fireline-harness` reduces to: approval gate, audit, budget, context injection, secrets injection, approval gate subscriber + other DurableSubscriber profiles. Pure session-execution concerns.

### R3. Fix `StateMaterializer` empty-stream behavior

Make preload either (a) tolerate empty stream and subscribe live, or (b) block for first envelope with a timeout. Remove the pre-seed ordering dependency in every boot path.

### R4. Consolidate event vocabulary

One canonical event set (`host.present`, `host.heartbeat`, `host.gone`) across both streams (or just the deployment stream, with the state stream reading projections). Drop `HostRegistered`+`HostProvisioned` pair; drop `host_stopped`+`host_deregistered`+`host_instance_stopped` triple. Fewer event kinds = fewer readers' projections to maintain.

### R5. Structured shutdown

Best-effort shutdown that always runs every step, aggregates errors. Add `impl Drop for BootstrapHandle` that best-effort-aborts tasks on abnormal exit.

### R6. Minor cleanups

- Drop unused `BootstrapConfig.control_plane_url`.
- Replace `Arc<dyn Fn() -> Vec<...>>` factory with typed list until a second consumer exists.
- Make `SharedTerminal::spawn` accept `&[String]` or `Arc<[String]>`.
- `connect_host()` → accept an optional `advertised_base_url` override.
- Make `BootstrapHandle` identity fields private + accessor methods.
- Verify `host_spec_persisted` target stream (agent vs hosts plane).

---

## 4. Sequencing recommendation

**Don't start during canonical-ids + DS rollout** — host layer changes would collide with Phase 6A (DeploymentSpecSubscriber) and any ongoing operator-script work.

**After canonical-ids Phase 8 closes:**

1. Beaded R1 (HostCore extraction) as a new epic `mono-host-core`. Same-project scope as DS Phase 6A or later.
2. R3 (materializer fix) is lowest-risk; could ship earlier as a standalone patch.
3. R2 (layering flip) is big but mechanical — move files, update imports, no behavior change.
4. R4 (event vocab consolidation) is a breaking change on the deployment stream — coordinate with any consumers.
5. R5/R6 are polish, land opportunistically.

**Demo impact**: none. Current hosting works for all tracked demo scenarios. This is post-demo technical debt cleanup.

---

## 5. What's actually good

To balance — the host layer has real strengths:

- **Durable-streams-as-truth is honored.** Boot emits to streams; readers project. No in-memory-only state that would vanish on crash.
- **Graceful shutdown is wired.** `oneshot::channel` + axum's `with_graceful_shutdown` is the right pattern.
- **Plane separation mostly held.** State stream carries agent-plane rows; deployment stream carries infra-plane rows. Some leakage (S11) but the model is there.
- **`StreamDeploymentPeerRegistry` is a real abstraction, not a dict+lock.** Discovery via stream read.
- **`control_plane.rs` is already tight** — 101 lines, delegates correctly to `router.rs` + dispatcher. The issue is symmetry with bootstrap, not control_plane itself.
- **PR #4 session/load fix + bb8cd9d HostIndex stale-Ready fix** landed cleanly despite the debt. The layer is fixable.

The debt is real but not a disaster. Canonical-ids Phase 8 closes the refactor; `HostCore` extraction is the next reasonable structural move.

## References

- `crates/fireline-host/src/bootstrap.rs`
- `crates/fireline-host/src/control_plane.rs`
- `crates/fireline-host/src/router.rs`
- `crates/fireline-harness/src/host_topology.rs`
- `crates/fireline-harness/src/routes_acp.rs`
- `crates/fireline-harness/src/trace.rs`
- [alignment-check-2026-04-12.md](./alignment-check-2026-04-12.md)
- [state-projector-audit-review-2026-04-12.md](./state-projector-audit-review-2026-04-12.md)
