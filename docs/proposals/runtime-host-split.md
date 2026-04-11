# Proposal: Split `RuntimeHost`

> Staging ground for item 2 of the architectural-debt report. **Proposal only — no code changes.** Informs a post-demo slice.

## Background

`RuntimeHost` (`crates/fireline-conductor/src/runtime/mod.rs:29`) is the single type the control plane talks to when it wants to create, register, heartbeat, stop, or delete a managed runtime. It was small at the start of Slice 04 and has accreted responsibilities since. Commit `6045c4a` ("Unify control-plane liveness ownership") already folded `HeartbeatTracker` **into** `RuntimeRegistry`, so liveness is clean. The remaining tangle is **launch vs. registration vs. durable event emission**.

## 1. Current state (what each method actually does)

Citations are to `crates/fireline-conductor/src/runtime/mod.rs`.

**`RuntimeHostInner` fields** (`:33–:38`):

- `registry: RuntimeRegistry` — TOML-persisted descriptors + liveness.
- `manager: RuntimeManager` — provider dispatch (Local | Docker).
- `live_handles: Mutex<HashMap<String, RuntimeLaunch>>` — live `Box<dyn ManagedRuntime>` handles the host must call `shutdown()` on.
- `pending_runtime_specs: Mutex<HashMap<String, PersistedRuntimeSpec>>` — workaround cache; see §2.

**`create(spec)`** (`:60–:166`) does all of:

1. Allocate `runtime_key` + `node_id` (`:61–:69`).
2. Upsert a `Starting` descriptor into the registry (`:73–:85`).
3. Dispatch to `manager.start(...)` and roll back registry+pending map on failure (`:87–:99`).
4. Insert the `PersistedRuntimeSpec` into `pending_runtime_specs` (`:100–:104`).
5. Park the `RuntimeLaunch` in `live_handles` (`:111–:115`).
6. If the registry row was already advanced by a concurrent `register()`, flush the pending spec and return (`:117–:139`).
7. Otherwise build a fresh `Starting` descriptor with launch endpoints, upsert, emit `emit_runtime_spec_persisted`, remove pending entry (`:141–:162`).

**`stop(key)`** (`:177–:198`): drain `live_handles`, `shutdown()`, flip registry to `Stopped`.

**`delete(key)`** (`:200–:212`): delegates to `stop` if live, then `registry.remove`.

**`register(key, registration)`** (`:215–:276`): load descriptor, gate on `Stopped`, compute `next_status` from current phase, merge advertised endpoints from the registering child, flush pending spec if `state.url` is now populated, upsert.

**`heartbeat(key, report)`** (`:278–:301`): load, gate on `Stopped`, lift `Stale → Ready`, stamp `updated_at_ms`.

The visible roles `RuntimeHost` is playing at once: **registry-writer**, **provider-launcher**, **live-handle-owner**, **durable-event-emitter**, **push-mode registration state machine**, **heartbeat reactor**.

## 2. Pressure points

**The `pending_runtime_specs` cache is a patch, not a cure.** Its origin (handoff-2026-04-11-managed-agent-suite.md:108):

> "it exposed a pre-existing race in `RuntimeHost::create` where the runtime_spec envelope could be skipped if the runtime registered before `emit_runtime_spec_persisted` was reached. That race was fixed separately in `b6156d9` (pending_runtime_specs map)."

The fix is correct but the shape of the fix is the smell: `create()` and `register()` **both** race to read the same map, **both** read `descriptor.state.url` to decide whether they can flush, and **both** call `emit_runtime_spec_persisted` (`mod.rs:130`, `:156`, `:261`). Three call sites, two acquire/release cycles on the same Mutex inside a single `create()` call (`:100`, `:122`, `:132`, `:159`), and the invariant ("every create emits exactly one `runtime_spec_persisted` regardless of which path gets there first") lives only in the reviewer's head.

**The `live_handles` map is not lifecycle-aware of the registry.** If a `Stopped` descriptor and an absent `live_handles` entry get out of sync — for example, a provider failure that leaves the child running but `stop` returning `"not running"` (`:184`) — the only recovery is a process restart. There is nothing a unit test can target because the two stores live inside the same type behind `Mutex`es.

**There is no place to put re-emission logic.** If Fireline ever needs to re-emit `runtime_spec_persisted` on cold-restart (e.g., after the state stream's snapshot was pruned), it has nowhere sensible to land. `RuntimeHost::create` is the only code path that emits today, and re-emission isn't a create.

**Tests have to drive full end-to-end lifecycles** to exercise registration-state-machine branches, because the branches are intertwined with provider dispatch and registry IO.

## 3. Proposed split — three concerns, three types

The cut that falls out of the pressure points:

### 3a. `RuntimeLauncher` — owns `live_handles`, drives providers

```rust
// crates/fireline-conductor/src/runtime/launcher.rs  (new)
pub struct RuntimeLauncher {
    manager: RuntimeManager,
    live_handles: Mutex<HashMap<String, RuntimeLaunch>>,
}

impl RuntimeLauncher {
    pub fn new(manager: RuntimeManager) -> Self;

    /// Dispatch to the provider; park the handle under `runtime_key`.
    /// Returns the launch endpoints so the caller can update a descriptor.
    pub async fn launch(
        &self,
        spec: CreateRuntimeSpec,
        runtime_key: &str,
        node_id: &str,
    ) -> Result<LaunchOutcome>;

    /// Take the parked handle and shut it down. Returns `Ok(None)` if absent.
    pub async fn shutdown(&self, runtime_key: &str) -> Result<Option<()>>;

    pub async fn is_live(&self, runtime_key: &str) -> bool;
}
```

One reason to change: **what provisioning looks like**.

### 3b. `RuntimeSpecJournal` — owns durable `runtime_spec_persisted` lifecycle

```rust
// crates/fireline-conductor/src/runtime/spec_journal.rs  (new)
pub struct RuntimeSpecJournal {
    pending: Mutex<HashMap<String, PersistedRuntimeSpec>>,
}

impl RuntimeSpecJournal {
    pub fn new() -> Self;

    /// Called at the start of `create` — before we know state.url.
    pub async fn record_pending(&self, runtime_key: &str, spec: PersistedRuntimeSpec);

    /// Called whenever we have both a runtime_key AND a non-empty state.url
    /// (from `create` after `manager.start`, or from `register` on late state URL).
    /// Idempotent: emits-and-clears at most once per key.
    pub async fn flush_if_ready(&self, runtime_key: &str, state_url: &str) -> Result<bool>;

    /// Called on create-rollback.
    pub async fn forget(&self, runtime_key: &str);
}
```

One reason to change: **how runtime_spec envelopes get into the state stream** (including future re-emission). The `emit_runtime_spec_persisted` helper currently in `crates/fireline-conductor/src/trace.rs:134` moves here; `trace.rs` stops being the owner of this side-effect.

Invariant becomes enforceable in one place: *"`flush_if_ready` either emits exactly once or remains pending."* That's unit-testable without spinning a provider.

### 3c. `RuntimeLifecycle` — owns descriptor state transitions

```rust
// crates/fireline-conductor/src/runtime/lifecycle.rs  (new)
pub struct RuntimeLifecycle {
    registry: RuntimeRegistry,
}

impl RuntimeLifecycle {
    pub fn new(registry: RuntimeRegistry) -> Self;

    /// create() step 2: insert the initial Starting row.
    pub fn begin_starting(&self, key: &str, node_id: &str, provider: RuntimeProviderKind) -> Result<RuntimeDescriptor>;

    /// create() step 7 / register(): merge launch/registration endpoints.
    pub fn apply_registration(&self, key: &str, reg: RuntimeRegistration) -> Result<RuntimeDescriptor>;

    /// heartbeat(): Stale → Ready, bump updated_at_ms.
    pub fn apply_heartbeat(&self, key: &str, report: HeartbeatReport) -> Result<RuntimeDescriptor>;

    /// stop(): flip to Stopped.
    pub fn mark_stopped(&self, key: &str) -> Result<RuntimeDescriptor>;

    /// rollback path for create()/delete().
    pub fn forget(&self, key: &str) -> Result<Option<RuntimeDescriptor>>;

    /// read-through helpers.
    pub fn get(&self, key: &str) -> Result<Option<RuntimeDescriptor>>;
    pub fn list(&self) -> Result<Vec<RuntimeDescriptor>>;
}
```

One reason to change: **the registration/heartbeat state machine**. This is the home for the `Starting → Ready`, `Stale → Ready`, `Stopped ⇒ reject` logic currently in `mod.rs:226–239` and `:295–297`.

### 3d. What remains of `RuntimeHost`

Two options — deferred to gnijor (see §5):

**(i) Thin façade.** `RuntimeHost { launcher, journal, lifecycle }` composes all three and keeps the public surface (`create`/`stop`/`delete`/`register`/`heartbeat`) for backward compatibility with `fireline-control-plane::router` and `src/bootstrap.rs` callers.

**(ii) Collapse.** Delete `RuntimeHost`; the control plane (`crates/fireline-control-plane/src/main.rs`) composes the three directly. More honest but a wider blast radius for a single slice.

## 4. Transition plan

Each commit individually reviewable, keeps `cargo test` green, and leaves `RuntimeHost`'s public API unchanged until the last step.

**Commit 1 — Extract `RuntimeSpecJournal`.** Move `pending_runtime_specs` off `RuntimeHostInner` into a new `runtime/spec_journal.rs`. `RuntimeHost` holds it as a field. Move `emit_runtime_spec_persisted` from `trace.rs` into the journal (`trace.rs` re-exports for one release). No behavior change; four `Mutex::lock` sites in `mod.rs` become three method calls. Add a unit test: two concurrent `flush_if_ready` calls on the same key emit exactly one envelope.

**Commit 2 — Extract `RuntimeLifecycle`.** Move `register`, `heartbeat`, `get`, `list`, the rollback branches, and the `Starting` upsert out of `RuntimeHost` into a new `runtime/lifecycle.rs`. `RuntimeHost` delegates. This is the commit where the registration state machine becomes directly unit-testable (no provider needed).

**Commit 3 — Extract `RuntimeLauncher`.** Move `live_handles`, the `manager.start` call, and `shutdown()` into a new `runtime/launcher.rs`. `RuntimeHost::create` is now a script: `lifecycle.begin_starting` → `journal.record_pending` → `launcher.launch` → `journal.flush_if_ready` → `lifecycle.apply_registration`. `stop`/`delete` become two-liners.

**Commit 4 — Push the composition boundary up.** Decide façade vs. collapse. If façade: `RuntimeHost::new` takes a `RuntimeManager` + `RuntimeRegistry`, builds the three parts inside. If collapse: change `fireline-control-plane::AppState` to hold the three directly and delete `RuntimeHost`. The former is lower-risk for a post-demo slice; the latter is the clean cut.

**Commit 5 (optional) — Add the missing invariants as assertions.** With the parts separated, write:
- A `cargo test` that races `create` and `register` for the same key and asserts exactly one `runtime_spec_persisted` envelope lands in the state stream (closes the `b6156d9` regression window with a test).
- A property-style test for `RuntimeLifecycle::apply_registration` covering every `RuntimeStatus` → `next_status` transition from `mod.rs:232–239`.

Each commit is individually revertable. Tests touching `RuntimeHost::create/stop/delete` (e.g., `tests/control_plane_push.rs`, `tests/runtime_provider_lifecycle.rs`) see no API change through commits 1–3.

## 5. Open questions (need gnijor input)

1. **Façade or collapse?** Keep `RuntimeHost` as a three-field composition struct, or delete it and have the control plane compose directly? Collapse is cleaner; façade is lower-risk before the demo.
2. **Direct-host mode ownership.** `src/bootstrap.rs` runs `fireline` in direct-host mode (not managed by a control plane) and doesn't go through `RuntimeHost::create` at all. Does the direct-host path need `RuntimeSpecJournal` too (i.e., should it emit `runtime_spec_persisted`?), or is the journal only relevant in push-mode? Today the answer is implicit; the split forces us to name it.
3. **Re-emission semantics.** If the durable stream is truncated/reset (there's already a `snapshot-start`/`reset` control protocol — see handoff-2026-04-11-managed-agent-suite.md:98–108), should `RuntimeSpecJournal` re-emit every known `PersistedRuntimeSpec`, and where does it get the list from? The registry has descriptors but not specs. This probably wants a follow-up proposal — flag, don't solve.
4. **Should `RuntimeLifecycle` own the `Mutex<HashMap>` liveness that moved into `RuntimeRegistry` in `6045c4a`?** Currently liveness is a registry concern; after this split, the "update `updated_at_ms`" action is a lifecycle concern. Two options: lifecycle calls through to registry (status quo), or the liveness map migrates up. Probably leave it in the registry — but note for review.
5. **Token issuance.** `RuntimeTokenIssuer` (`crates/fireline-conductor/src/runtime/provider.rs:316`) is consumed by `DockerProvider` but conceptually it's a launch-time concern. Does it belong on `RuntimeLauncher`, or is it fine where it is (passed through the manager)? Out of scope for this split, but the question becomes visible once the launcher exists.

## Appendix — Files touched by this proposal

| Concern | New file | Code moved from |
|---|---|---|
| Launch + live handles | `crates/fireline-conductor/src/runtime/launcher.rs` | `mod.rs:60–166`, `:177–212` |
| Spec journal | `crates/fireline-conductor/src/runtime/spec_journal.rs` | `mod.rs:37`, `:100–162`, `:251–267`; `trace.rs:134` |
| Lifecycle state machine | `crates/fireline-conductor/src/runtime/lifecycle.rs` | `mod.rs:73–85`, `:215–301` |
| Façade (if option (i)) | `crates/fireline-conductor/src/runtime/mod.rs` | (thinned from ~318 lines to ~80) |

No callers in `fireline-control-plane`, `fireline-components`, or `src/` need to change until **Commit 4** — and then only if option (ii) is chosen.
