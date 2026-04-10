# Runtime Registration and Heartbeat

> Status: design doc (not yet implemented)
> Type: reference doc for the push-based lifecycle model that replaces polling when non-local providers land
> Audience: maintainers implementing `DockerProvider`, `CloudflareProvider`, or any future provider whose runtimes do not share a filesystem with the control plane
> Related:
> - [`control-and-data-plane.md`](./control-and-data-plane.md) — §2, §4, §4a
> - [`../execution/13-distributed-runtime-fabric-foundation.md`](../execution/13-distributed-runtime-fabric-foundation.md)
> - `crates/fireline-control-plane/src/local_provider.rs` — current polling implementation
> - `src/main.rs` `run_managed_runtime()` — the runtime side of the current model

## Purpose

The phase 1 control plane (commit `33ef14e`) uses a **polling-based readiness model**: the control plane spawns a runtime subprocess, waits for the child to write `RuntimeStatus::Ready` into a shared `runtimes.toml` registry file, and then returns the descriptor to the caller. This works because the control plane and the spawned runtime share a filesystem.

It **silently breaks** the moment a provider runs runtimes in a place that doesn't share the control plane's filesystem:

- **Docker containers** — the container's view of `runtimes.toml` is a volume mount at best, or a disjoint path at worst.
- **Cloudflare workers** — no filesystem at all.
- **Remote machines** (SSH, Kubernetes pods) — different host, no shared storage.

The push-based model in this doc is the replacement that makes cross-host providers work correctly. It is deliberately **not yet implemented**; the current polling path is sufficient for the phase 1 local-only target, and the e2e test (`b3cc3f7`) depends on it. This doc exists so that when the `DockerProvider` build starts — which is the first provider that needs push — the protocol, state machine, and invariants are already pinned.

## Context: why polling works for local

From `ChildProcessRuntimeLauncher::wait_for_runtime_ready`:

```rust
loop {
    if let Some(runtime) = self.runtime_registry.get(runtime_key)? {
        if runtime.status == RuntimeStatus::Ready {
            return Ok(runtime);
        }
    }
    if let Some(status) = child.try_wait()? {
        return Err(anyhow!("fireline runtime exited before becoming ready: {status}"));
    }
    if tokio::time::Instant::now() >= deadline {
        return Err(anyhow!("timed out waiting for runtime '{runtime_key}' to become ready"));
    }
    tokio::time::sleep(self.poll_interval).await;
}
```

The control plane reads the shared `runtime_registry` file every 100ms. The spawned runtime binary, in `run_managed_runtime()`, finishes bootstrap and then calls `registry.upsert(descriptor)` with `status: Ready`. The next poll cycle observes the new row and returns.

Both sides must be able to open the same file. That's the entire mechanism.

### What polling gets right

- **No network dependency between control plane and runtime.** If the control plane can spawn the process, it can observe readiness. No HTTP client in the runtime binary, no reverse call path.
- **Race-free readiness detection.** The runtime writes `Ready` *after* its listeners are bound and serving; the control plane sees `Ready` *after* the write; no window exists where `Ready` is visible but connections are refused.
- **Child-exit detection is immediate.** `child.try_wait()` catches crashes before the bootstrap timeout fires, so failures are reported as "exited before ready" rather than "timed out."
- **Simple to reason about.** One shared file, one poll loop, no state machine.

### Why polling can't survive

- **Filesystem coupling.** The control plane and every runtime must share storage. Violated by containers, VMs, remote hosts, serverless.
- **No liveness signal after ready.** Once a runtime is marked `Ready`, polling never re-checks. A runtime that wedges — listener still bound, connections accepted, but nothing responding — stays `Ready` forever. The only failure signal is `child.try_wait()`, which only fires on actual process exit.
- **No control-plane-restart recovery.** Phase 1 keeps the registry in-memory. If the control plane restarts, its view of "which runtimes exist and are ready" is lost; the runtime keeps running but the control plane has no record. There's no re-registration path.
- **Ambiguity for cross-host.** If two nodes share nothing, "the registry" is either duplicated (two truths) or centralized (back to the control plane, but then you need a push path to put data into it, which defeats the point).

Polling is a **local optimization** masquerading as a lifecycle protocol. It works for phase 1's target (one control plane + N local subprocesses on one machine) and only that.

## The push-based alternative

Replace the file-based signal with two HTTP endpoints on the control plane, plus a lightweight HTTP client inside the runtime binary.

### Endpoints (on the control plane)

These are additive to the phase 1 surface defined in `control-and-data-plane.md` §2. They live on the same axum router, same auth boundary.

```text
POST /v1/runtimes/{runtimeKey}/register    body: RuntimeRegistration  → 200 OK / 401 / 409
POST /v1/runtimes/{runtimeKey}/heartbeat   body: HeartbeatReport      → 200 OK / 401 / 410
```

**`POST /v1/runtimes/{runtimeKey}/register`** is what a runtime calls exactly once at startup, after it has bound its ACP listener and verified it is accepting connections, but before it begins serving real traffic (see §"Ordering at runtime startup" below).

```rust
pub struct RuntimeRegistration {
    pub runtime_id: String,
    pub node_id: String,
    pub provider: RuntimeProviderKind,
    pub provider_instance_id: String,
    pub advertised_acp_url: String,
    pub advertised_state_stream_url: String,
    pub helper_api_base_url: Option<String>,
    pub capabilities: RuntimeCapabilities, // e.g. load_session, peer, topology components
}
```

The control plane handles it as an **upsert keyed by `runtime_key`**:

- If no record exists, create one with `status: Ready` and the provided advertised endpoints.
- If a record exists with `status: Starting`, transition it to `Ready` and merge the registration fields.
- If a record exists with `status: Ready`, treat this as a re-registration (e.g. after control plane restart, or after the runtime's own restart with the same `runtime_key`). Update the fields and return 200. This is the path that makes control-plane-restart transparent.
- If a record exists with `status: Stopped`, return 409 — the control plane thinks this runtime is done, the runtime disagrees, operator intervention needed.

**`POST /v1/runtimes/{runtimeKey}/heartbeat`** is what a runtime calls on a 5-second cadence for the rest of its life.

```rust
pub struct HeartbeatReport {
    pub ts_ms: i64,
    pub metrics: Option<HeartbeatMetrics>, // load hints: active_sessions, queue_depth, etc.
}
```

The control plane records the timestamp of the most recent heartbeat per `runtime_key` and treats it as a liveness indicator (see §"Status state machine").

**No response body** on either endpoint beyond the HTTP status. Heartbeats are fire-and-forget; registration returns the updated descriptor if it's useful (optional — v1 can return empty 200).

### Cadence and timeouts

| Setting | Default | Rationale |
|---|---|---|
| Heartbeat period | **5 seconds** | Low enough to catch wedges within the stale threshold; high enough that 1000 runtimes heartbeating cost <1 QPS/runtime |
| Stale timeout | **30 seconds** | 6x the heartbeat period — three consecutive missed heartbeats before the runtime is considered stale |
| Registration timeout | **2 seconds per attempt** | The runtime must make progress; a hanging control plane should not stall runtime startup forever |
| Registration retry backoff | **250ms → 2s, 3 attempts** | Transient network blips shouldn't fail-fast; repeated failures should fail startup so the launcher can react |

These are defaults, not contracts. A Docker provider running fleets of runtimes might choose 10s heartbeats to reduce CP traffic; a development setup might use 1s for snappier failure detection. The **control plane** owns the stale threshold and can be configured per environment.

### Authentication

Every `/register` and `/heartbeat` request carries `Authorization: Bearer <token>`. The token is issued by the control plane to the launcher when the launcher creates the runtime (via the existing `/v1/auth/runtime-token` endpoint mentioned in the architectural doc §2), and is passed to the runtime via the `FIRELINE_CONTROL_PLANE_TOKEN` env var. The runtime includes it on every call.

Token scope: write-only for `/register` and `/heartbeat` on the runtime's **own** `runtime_key`. A runtime cannot register or heartbeat for a different key. This is enforced server-side on the control plane, not by the runtime.

## Status state machine

The lifecycle under the push model is a formal state machine:

```
                     create()
                        │
                        ▼
                   ┌──────────┐
                   │ starting │        no heartbeat yet, registration not received
                   └─────┬────┘
                         │
           register() ◄──┤
                         │
                         ▼
                   ┌──────────┐
         ┌─────────│  ready   │───────┐
         │         └─────┬────┘       │
         │               │            │
         │ 30s w/o       │ heartbeat  │ stop()
         │ heartbeat     │ (normal)   │
         │               │            │
         ▼               ▼            ▼
    ┌─────────┐     ┌────────┐   ┌─────────┐
    │  stale  │     │ (loop) │   │ stopping│
    └────┬────┘     └────────┘   └────┬────┘
         │                             │
         │ heartbeat resumes           │ shutdown complete /
         │                             │ child exit observed
         ▼                             ▼
    ┌─────────┐                   ┌─────────┐
    │  ready  │                   │ stopped │
    └─────────┘                   └─────────┘

                   ┌──────────┐
                   │  broken  │  ← child exit (via launcher signal)
                   └──────────┘     or /register returns 409
                                    or explicit "I'm shutting down unexpectedly" heartbeat
```

**Transitions:**

| From | To | Trigger | Who observes / writes |
|---|---|---|---|
| `starting` | `ready` | First successful `/register` call | Control plane handles the POST |
| `starting` | `broken` | Registration timeout exceeded, or launcher reports child exit | Control plane (internal timeout or launcher signal) |
| `ready` | `ready` | Heartbeat within stale threshold | Control plane updates `last_heartbeat_at` |
| `ready` | `stale` | No heartbeat for 30s (stale threshold) | Control plane (scheduled check) |
| `ready` | `stopping` | `POST /v1/runtimes/{key}/stop` from a client | Control plane API handler |
| `ready` | `broken` | Launcher reports child exit while state was `ready` | Control plane (launcher signal) |
| `stale` | `ready` | Next successful heartbeat | Control plane handles the heartbeat |
| `stale` | `broken` | Launcher reports child exit, or operator action | Control plane (launcher signal or manual) |
| `stopping` | `stopped` | Launcher confirms shutdown complete | Control plane (launcher signal) |

**Terminal states:** `stopped`, `broken`. Neither transitions further without explicit operator action (delete + recreate).

**Queryable states:** all of them. The §4a invariants still apply — only `ready` is a promise that data-plane connections will succeed. Consumers reading a descriptor whose status is `starting`, `stale`, `broken`, `stopping`, or `stopped` must not open `/acp` or `/v1/stream/*`.

## Backward compatibility with the polling model

This section is the migration plan.

### Slice 13 v1 (current): LocalProvider + polling

- `LocalProvider` uses `ChildProcessRuntimeLauncher`, which uses the polling model.
- Runtimes write `Ready` directly to the shared `runtime_registry` file.
- No `/register` or `/heartbeat` endpoints on the control plane.
- No `control_plane_client` in the runtime binary.

**Status:** shipped in commit `33ef14e`. Works for local-only deployments. This doc does not ask us to change it.

### Slice 13 v2 (proposed): add endpoints, make LocalProvider opt-in to push

- Control plane grows `/register` and `/heartbeat` endpoints, plus an in-memory `heartbeat_tracker` that records last-heartbeat-per-runtime and a scheduled task that transitions stale runtimes.
- Runtime binary grows `src/control_plane_client.rs` containing a small HTTP client wrapper around `reqwest`, with retry + backoff.
- A new CLI flag `--control-plane-url <URL>` on the runtime binary switches it from "write to file" mode to "call HTTP endpoints" mode. When the flag is present, the runtime does NOT write to the registry file — the control plane is the sole source of truth for this runtime's existence.
- `LocalProvider` gets a config switch: `prefer_push: bool`. When false (default for backwards compat), spawns runtimes in polling mode. When true, spawns them with `--control-plane-url` set.
- The e2e test in `packages/browser-harness` continues to pass against both modes (push and polling) — the browser only sees descriptors, not the mechanism.

**Status:** not yet started. This is what the doc is the blueprint for.

### Slice 13 v3 (implied): DockerProvider requires push

- `DockerProvider` spawns a container and exposes the runtime inside. The containerized runtime has no shared filesystem with the control plane.
- `DockerProvider`'s launcher always sets `--control-plane-url` on the containerized runtime's CLI (or `FIRELINE_CONTROL_PLANE_URL` env var). Polling is not an option.
- If `--control-plane-url` is absent on a runtime spawned by `DockerProvider`, the launcher fails fast with a clear error.

**Status:** blocked on v2.

### Slice 13 v4 (implied): LocalProvider defaults to push

- Once v2 is stable and all tests pass against both modes, flip `LocalProvider::prefer_push` default to `true`.
- Polling mode remains as a legacy fallback (or is removed entirely in a later cleanup).
- This closes the loop — every provider uses push, polling is historical.

**Status:** blocked on v3.

## Runtime-side client sketch

Where `src/control_plane_client.rs` eventually lands in the binary crate. This is pseudocode with fully-qualified types and the exact shape; not meant to be copy-pasted but close enough that an implementer can wire it directly.

```rust
//! Control-plane HTTP client used by runtime binaries running in push mode.
//!
//! This client is only constructed when `--control-plane-url` is present
//! on the runtime CLI. In polling mode (the phase-1 default) the runtime
//! writes directly to the shared runtime_registry file and this module
//! is not used.

use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::{Client as HttpClient, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

use fireline_conductor::runtime::{RuntimeProviderKind, RuntimeCapabilities};

#[derive(Clone)]
pub struct ControlPlaneClient {
    http: HttpClient,
    base_url: String,
    token: String,
    runtime_key: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRegistration {
    pub runtime_id: String,
    pub node_id: String,
    pub provider: RuntimeProviderKind,
    pub provider_instance_id: String,
    pub advertised_acp_url: String,
    pub advertised_state_stream_url: String,
    pub helper_api_base_url: Option<String>,
    pub capabilities: RuntimeCapabilities,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatReport {
    pub ts_ms: i64,
    pub metrics: Option<HeartbeatMetrics>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatMetrics {
    pub active_sessions: u32,
    pub queue_depth: u32,
}

impl ControlPlaneClient {
    pub fn new(
        base_url: impl Into<String>,
        token: impl Into<String>,
        runtime_key: impl Into<String>,
    ) -> Self {
        Self {
            http: HttpClient::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .expect("reqwest client"),
            base_url: base_url.into(),
            token: token.into(),
            runtime_key: runtime_key.into(),
        }
    }

    /// Register this runtime with the control plane. Called exactly
    /// once at startup after listeners are bound. Retries with
    /// exponential backoff on transient failures.
    pub async fn register(&self, registration: RuntimeRegistration) -> Result<()> {
        let url = format!(
            "{}/v1/runtimes/{}/register",
            self.base_url.trim_end_matches('/'),
            self.runtime_key
        );
        let mut backoff = Duration::from_millis(250);
        for attempt in 0..3 {
            let result = self
                .http
                .post(&url)
                .bearer_auth(&self.token)
                .json(&registration)
                .send()
                .await;
            match result {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                Ok(resp) if resp.status() == StatusCode::CONFLICT => {
                    return Err(anyhow::anyhow!(
                        "control plane rejected registration with 409; runtime_key conflict"
                    ));
                }
                Ok(resp) => {
                    tracing::warn!(
                        attempt,
                        status = %resp.status(),
                        "control-plane registration attempt failed"
                    );
                }
                Err(error) => {
                    tracing::warn!(attempt, ?error, "control-plane registration error");
                }
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(2));
        }
        Err(anyhow::anyhow!("control plane registration failed after 3 attempts"))
    }

    /// Spawn a background task that heartbeats every 5 seconds until
    /// the returned JoinHandle is aborted. Best-effort: single
    /// heartbeat failures are logged but do not stop the loop.
    pub fn spawn_heartbeat_loop(
        self: &std::sync::Arc<Self>,
        metrics_source: impl Fn() -> HeartbeatMetrics + Send + 'static,
    ) -> JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            let url = format!(
                "{}/v1/runtimes/{}/heartbeat",
                this.base_url.trim_end_matches('/'),
                this.runtime_key
            );
            loop {
                let report = HeartbeatReport {
                    ts_ms: now_ms(),
                    metrics: Some(metrics_source()),
                };
                let result = this
                    .http
                    .post(&url)
                    .bearer_auth(&this.token)
                    .json(&report)
                    .send()
                    .await;
                if let Err(error) = result.and_then(|resp| resp.error_for_status()) {
                    tracing::warn!(?error, "control-plane heartbeat failed");
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        })
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
```

Where it gets called in `src/main.rs` `run_managed_runtime()`:

```rust
// After bootstrap completes and listeners are bound:
if let Some(control_plane_url) = cli.control_plane_url {
    let token = std::env::var("FIRELINE_CONTROL_PLANE_TOKEN")
        .context("FIRELINE_CONTROL_PLANE_TOKEN required in push mode")?;
    let client = std::sync::Arc::new(ControlPlaneClient::new(
        control_plane_url,
        token,
        runtime_key.clone(),
    ));
    client
        .register(RuntimeRegistration {
            runtime_id: handle.runtime_id.clone(),
            node_id: node_id.clone(),
            provider: RuntimeProviderKind::Local,
            provider_instance_id: handle.runtime_id.clone(),
            advertised_acp_url: handle.acp_url.clone(),
            advertised_state_stream_url: handle.state_stream_url.clone(),
            helper_api_base_url: None,
            capabilities: detect_capabilities(&topology),
        })
        .await?;
    let _heartbeat_task = client.spawn_heartbeat_loop(|| HeartbeatMetrics {
        active_sessions: 0, // TODO: thread real metrics through
        queue_depth: 0,
    });
    // ... run until SIGINT ...
} else {
    // polling mode (existing path): write to runtime_registry file directly
    registry.upsert(descriptor.clone())?;
    // ... run until SIGINT ...
}
```

## Ordering at runtime startup

This is the part that preserves the §4a readiness invariants. The sequence is **not negotiable**; any deviation breaks rule 2 ("`ready` is a data-plane promise").

1. Parse CLI args, load config, initialize logging.
2. Bring up the durable-streams producer (embedded or external).
3. **Bind the axum listener.** `/acp`, `/healthz`, helper API all become reachable.
4. **Verify the listener is accepting connections.** A self-check `GET /healthz` from inside the runtime process, or an explicit "am I up" signal via the tokio runtime. This eliminates the window where the port is bound but the server isn't processing requests.
5. Start the runtime-local materializer subscription and preload.
6. **Call `ControlPlaneClient::register()`.** This is the moment `ready` becomes visible to consumers. If registration fails, the runtime shuts down its listeners cleanly and exits with an error — it does not remain running with no control-plane record.
7. Start the heartbeat loop.
8. Begin serving traffic until SIGINT / shutdown signal.

The critical invariant: **no consumer can see `status: ready` for this runtime until step 6 completes successfully, which itself only happens after steps 3–5 complete successfully.** That preserves rule 2.

## Invariant preservation vs §4a

Re-checking each of the §4a readiness invariants against the push model:

**1. The control plane owns runtime existence.**
✅ Stronger under push than under polling. In polling mode, a runtime can write itself into the registry file and exist to consumers without the control plane ever knowing. Under push, the control plane is the only writer of descriptor records; if a runtime doesn't register, it doesn't exist.

**2. `ready` is a data-plane promise, not a process-state hint.**
✅ Preserved by the ordering in §"Ordering at runtime startup". Registration happens after listener bind + self-check, so `ready` implies connections are accepted.

**3. Frontends must not probe the data plane speculatively.**
✅ Unchanged — this rule is about the client side and is independent of how the control plane computes `ready`.

**4. Dev-mode proxies do not change the contract.**
✅ Unchanged for the same reason.

**5. The control plane returns discovery material; the data plane carries work.**
✅ Unchanged. Register and heartbeat carry metadata (endpoints, health), not session payloads.

Additionally, the push model gives us a **new invariant** that polling couldn't provide:

**6. A runtime in `stale` or `broken` status is not consuming traffic.**
Under push, a wedged runtime stops heartbeating and is marked `stale` within 30 seconds. Consumers reading a stale descriptor must not attempt data-plane connections — adding `stale` to the "don't connect" list extends the set of protected states beyond `starting` / `stopped`.

This should land as an addition to §4a once the push model is implemented. Draft phrasing:

> 6. **`stale` and `broken` are not-ready states.** A runtime that has missed its heartbeat threshold or whose provider reports failure does not satisfy rule 2's "data-plane promise." Consumers must treat these the same as `starting` for the purpose of deciding whether to open `/acp` or state-stream subscriptions.

## Open questions deferred to implementation

These do **not** need to be decided before the implementation PR is drafted, but they will need answers somewhere in the implementation:

1. **Control plane's stale-check cadence.** The control plane needs a scheduled task that periodically scans the heartbeat tracker and transitions runtimes to `stale`. Every 5s? Every 1s? Per-runtime timer vs. one global scanner? Tradeoff is CPU cost on the CP vs. maximum detection latency.

2. **Heartbeat failure granularity.** If a single heartbeat POST fails (network blip), should the runtime log and keep going, or escalate after N consecutive failures? Default sketched above: log and keep going. No escalation — the control plane's stale timeout is the only thing that declares a runtime unreachable, not the runtime itself.

3. **Manual reset of `broken` runtimes.** Once a runtime is marked `broken`, does it stay broken forever, or can an operator reset it via a control-plane API? For v1, operator action = delete + recreate. Adding a `/v1/runtimes/{key}/reset` endpoint is a follow-up.

4. **Capabilities evolution.** `RuntimeCapabilities` in the registration body is currently a sketch. What concrete fields? `supports_load_session: bool`, `topology_components: Vec<String>`, `peer_enabled: bool`? Should probably map 1:1 to the fields the TS consumers already query on the descriptor.

5. **Metrics fidelity in heartbeats.** `HeartbeatMetrics { active_sessions, queue_depth }` is a placeholder. Real fields TBD based on what ops actually wants to observe. Start minimal and grow only when there's a dashboard demanding it.

6. **Token rotation.** The control plane issues a token at runtime-create time. What happens when the token expires mid-lifetime? Two options:
   - **Long-lived tokens** (hours or days). Simple, but a leaked token is bad.
   - **Short-lived tokens with refresh** (minutes, with a refresh endpoint). More secure, more plumbing.
   The phase 1 auth story (see `control-and-data-plane.md` §9) is "shared-secret bearer, format open." Token rotation is almost certainly a follow-up slice, not a v2 addition.

7. **Re-registration semantics for the same `runtime_key`.** If a runtime process dies and is respawned with the same `runtime_key` (e.g., container restart), does the control plane treat the new registration as "same runtime, new process" or "this is a conflict"? Currently specified as "upsert, update fields" but this collapses the distinction between "orderly restart" and "impersonation." Worth revisiting when auth hardens.

8. **Graceful shutdown semantics.** When a runtime receives SIGTERM, should it proactively POST to tell the control plane it's going down, or just stop heartbeating and let the stale timer fire? Proactive is nicer (faster observability) but adds a failure mode (what if the POST fails during shutdown?). Polling model has no analog; push model should support proactive notification via a simple "final heartbeat" with a `shutting_down: true` flag, falling back to stale detection if that POST fails.

## Migration plan (concrete steps)

For whoever picks up the implementation PR:

1. **Add endpoints to `fireline-control-plane`.**
   - New module `src/heartbeat.rs` with the `HeartbeatTracker` type + stale-check task
   - New routes in `src/main.rs` `Router::new()...`:
     - `POST /v1/runtimes/{runtime_key}/register`
     - `POST /v1/runtimes/{runtime_key}/heartbeat`
   - Handler functions call `RuntimeHost::register(key, registration)` and `RuntimeHost::heartbeat(key, report)` — new methods on `RuntimeHost` that operate on the in-memory registry
   - Background task (or tokio interval) that runs every 5s, iterates the heartbeat tracker, and transitions stale runtimes

2. **Add `RuntimeHost::register` and `::heartbeat`** methods in `fireline-conductor::runtime::mod`. Both are upsert-shaped; register transitions `starting → ready`, heartbeat updates `last_heartbeat_at`.

3. **Add `src/control_plane_client.rs`** in the binary crate with the sketch above.

4. **Add CLI flag** `--control-plane-url` and env var `FIRELINE_CONTROL_PLANE_URL` + `FIRELINE_CONTROL_PLANE_TOKEN` to the runtime binary. When present, `run_managed_runtime()` uses the push client instead of writing to the registry file.

5. **Update `LocalProvider`** in `fireline-control-plane` to optionally spawn runtimes with `--control-plane-url` pointed at the control plane's own listener. Add a `prefer_push: bool` config flag defaulting to `false` for phase 1 compatibility. When `true`, the launcher does NOT poll the registry file; it waits for the runtime's own `/register` call to transition the record to `ready`, with the same startup timeout semantics.

6. **Extend the e2e test** (`packages/browser-harness`) to run against both polling and push modes, parameterized by an env var. Verify the browser sees the same readiness invariants either way.

7. **Document the stale/broken state additions to §4a** of `control-and-data-plane.md` as rule 6 (draft phrasing above).

8. **Flip `LocalProvider::prefer_push` default to `true`** as a follow-up PR once v2 is stable and Docker provider work is ready to consume it.

## When this doc gets updated

If any of the following change, this doc should be edited (not superseded):

- Heartbeat cadence or stale threshold defaults
- The registration body shape (new fields, renamed fields)
- The state machine (new states, transitions, terminal states)
- Authentication changes (token format, rotation, scope)
- The ordering at runtime startup

If a broader change invalidates the push model itself (unlikely but possible — e.g., a move to streaming SSE-based heartbeats, or a move to gRPC), this doc should be superseded by a successor that cites it.
