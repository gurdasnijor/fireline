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

---

## 6. Interaction with stream-as-truth (post-decision)

> **Status:** this section supersedes parts of §3, §4, and §5 in light of a directional decision made after the original proposal was written. The prior sections are preserved unedited for history; read §6 as the currently-authoritative overlay.

### 6.1 The decision

gnijor validated a **stream-as-truth** direction in conversation on **2026-04-11** during the debt-paydown session. Commits **A + B** from workspace:4 landing shortly will implement it. The substance:

- **Delete the in-memory `RuntimeRegistry` entirely.** The TOML-backed `crates/fireline-conductor/src/runtime/registry.rs` store stops being canonical.
- **Runtime existence is derived from the durable stream.** Envelopes like `runtime_spec_persisted`, `runtime_stopped`, and the existing registration-state envelopes are the only source of truth for "does this runtime exist, and what is its current status?"
- **Heartbeats become optional liveness hints**, not state. A missed heartbeat downgrades a derived status; it does not mutate a stored row.
- **The control plane becomes a stateless reader** that materializes a `RuntimeIndex` projection from the stream — structurally identical to how `SessionIndex` / `ActiveTurnIndex` already work today (`src/session_index.rs`, `src/active_turn_index.rs`), driven by `RuntimeMaterializer` (`src/runtime_materializer.rs`).

This is **simplifying** to the split proposal, not competing. Every concern §3 extracted is still a concern — but two of the three shrink, and the hardest open question dissolves.

### 6.2 Updates to §3 — what each extracted concern becomes

#### §3a `RuntimeLauncher` — *survives cleanly (unchanged)*

**After stream-as-truth:** `RuntimeLauncher` is about owning a `Box<dyn ManagedRuntime>` and calling `shutdown()` on it. That is **subprocess fate**, which is orthogonal to "what is a runtime." The launcher still:

- Dispatches to `RuntimeManager` (Local | Docker).
- Parks the live handle under `runtime_key` in an in-process map.
- Exposes `launch`, `shutdown`, `is_live`.

The only adjustment: after a successful `launch()`, the launcher (or its caller) appends a `runtime_spec_persisted` envelope to the stream directly. Today `create()` does this as step 7; after the split + stream-as-truth, it is a single unconditional append — no pending cache, no race, no branching on a registry row.

**Unit-testability is unchanged:** the launcher can still be exercised with a fake `RuntimeManager`.

#### §3b `RuntimeSpecJournal` — *dissolves*

**After stream-as-truth:** there is nothing to reconcile. The `pending_runtime_specs` cache exists **only** because the original code raced the act of "learning `state.url` from a registering child" against the act of "emitting `runtime_spec_persisted` to that same `state.url`." Stream-as-truth collapses both into a single write: the launcher knows the `state.url` at the moment `manager.start()` returns (via `RuntimeLaunch.state.url`, see `mod.rs:108`), and there is no separate registry row to race against.

The proposed `runtime/spec_journal.rs` file is **not created**. The `emit_runtime_spec_persisted` helper at `crates/fireline-conductor/src/trace.rs:134` either stays in `trace.rs` or migrates onto `RuntimeLauncher` as a private method — neither placement needs the concept of "pending emission."

The invariant "every create emits exactly one `runtime_spec_persisted`" moves from "unit-testable on the journal" to "unconditional linear code on the launcher," which is strictly cheaper to reason about.

#### §3c `RuntimeLifecycle` — *shrinks from state-machine owner to stream writer*

**After stream-as-truth:** the `Starting → Ready`, `Stale → Ready`, and `Stopped ⇒ reject` transitions currently in `mod.rs:232–239` and `:295–297` are **no longer in-memory state transitions**. They become:

- **Stream writes** — "append a `runtime_registration` or `runtime_status_change` envelope describing the intended transition."
- **Projection reads** — the control plane's `RuntimeIndex` projection applies those envelopes deterministically on replay to produce the current `RuntimeStatus`.

What remains of `RuntimeLifecycle` is thin enough that it may not deserve its own file:

- A small module of stream-write helpers: `write_registration(...)`, `write_heartbeat(...)`, `write_stopped(...)`. Each is one `append_json(...)` call plus a `timestamp_ms`.
- A `RuntimeIndex` projection type in `src/runtime_index.rs` (new, mirroring `session_index.rs` in layout and traits), implementing `StateProjection` (`src/runtime_materializer.rs:56`) and exposing `get(key)`, `list()`.

The state machine's validity is enforced by **projection logic**, not by `Mutex` + early-return. Tests become property-style: "feed this envelope sequence into the projection, assert the resulting status is X" — the same pattern already in use for session/approval semantics in `fireline-semantics`.

### 6.3 Closing open question §5.3 — re-emission semantics

**Resolved.** The question asked where re-emission should live if the stream is truncated or reset. Under stream-as-truth, "re-emit" is the wrong verb. The stream is the primary record; recovery semantics are **replay**, which is already implemented by `RuntimeMaterializer` (`src/runtime_materializer.rs:91`, `offset: Beginning`, `live: Sse`).

If a `snapshot-start`/`reset` control message arrives (handoff-2026-04-11-managed-agent-suite.md:98–108), every projection including the new `RuntimeIndex` calls `StateProjection::reset` and rebuilds from the subsequent snapshot — identically to `SessionIndex` and `ActiveTurnIndex` today.

**There is no runtime-spec re-emission code to write.** The open question is not "postponed" — it ceases to exist.

### 6.4 Reframed §4 transition plan

The §4 sequence is premised on a world where `RuntimeRegistry` still owns canonical state. Stream-as-truth changes the starting baseline. The revised sequence:

**Commits A + B (workspace:4, landing first — not part of this split).** Introduce the `RuntimeIndex` projection + `runtime_spec_persisted` / `runtime_status_change` envelope schema; switch the control plane to read from the projection; deprecate writes to `RuntimeRegistry` behind a feature flag. After these land, `RuntimeHost` still exists but its `registry.upsert(...)` calls are dead weight — the control plane ignores them.

**Commit 1 (was §4 Commit 3) — Extract `RuntimeLauncher`.** Move `live_handles`, `manager.start`, and `shutdown()` into `crates/fireline-conductor/src/runtime/launcher.rs`. `RuntimeHost::create` is thinned to: allocate key/node → `launcher.launch(...)` → append `runtime_spec_persisted`. `stop`/`delete` become two-liners over `launcher.shutdown` + a stream append.

**Commit 2 (new) — Delete the in-memory state machine.** Remove `register`, `heartbeat`, and the `Starting`/`Stale`/`Ready` branching from `mod.rs:215–301`. Replace with thin stream-write helpers (§6.2 §3c). The control plane's `/register` and `/heartbeat` handlers start calling these helpers directly.

**Commit 3 (new) — Delete `RuntimeRegistry` and `RuntimeHostInner.live_handles` duplication.** With the registry gone and the launcher owning handles, `RuntimeHost` is either a one-field façade (`launcher`) or can be deleted entirely in favor of direct composition in `fireline-control-plane::AppState`.

**Commit 4 (optional, unchanged) — Add race-regression tests.** Same as original §4 Commit 5, but the test now asserts that a single `runtime_spec_persisted` envelope lands per `create()`, full stop — no Mutex, no pending cache, no branching.

**What drops out of the original plan:** the §4 Commit 1 (RuntimeSpecJournal extraction) is deleted — the journal doesn't exist under §6.2. The §4 Commit 2 (RuntimeLifecycle extraction) is replaced by Commit 2 above, which is a **delete** rather than an extraction.

Net: the split becomes smaller, not larger, because stream-as-truth removes concerns instead of relocating them.

### 6.5 New open question §5.6

**6. Do we need `RuntimeLifecycle` at all, or does it collapse into a projection plus the launcher's subprocess fate?** If every registration / heartbeat / stop becomes a stream append, and every status read becomes a projection lookup, then `RuntimeLifecycle` as a named type has no behavior worth naming — it's a folder of free functions. Two options:

- **(a) Keep the name.** A `RuntimeLifecycle` module holds the stream-write helpers as a discoverable API surface; the `RuntimeIndex` projection lives beside it. Low cost, mild ceremony.
- **(b) Drop the name.** Stream-write helpers become private functions in `fireline-control-plane::router` (where they are called from); `RuntimeIndex` lives in `src/runtime_index.rs` next to the other projections. Maximum honesty about what has survived.

Leaning (b) but flagging for review — the decision affects the appendix file list and whether "`RuntimeLifecycle`" appears anywhere in the codebase after the split lands.

### 6.6 Revised appendix — files touched after stream-as-truth

| Concern | File | Status |
|---|---|---|
| Launch + live handles | `crates/fireline-conductor/src/runtime/launcher.rs` | **New** (unchanged from original proposal) |
| Spec journal | ~~`crates/fireline-conductor/src/runtime/spec_journal.rs`~~ | **Dissolved** — not created |
| Lifecycle state machine | ~~`crates/fireline-conductor/src/runtime/lifecycle.rs`~~ | **Dissolved or demoted** — see §6.5 |
| Runtime index projection | `src/runtime_index.rs` | **New** (mirrors `session_index.rs`) |
| `RuntimeRegistry` TOML store | `crates/fireline-conductor/src/runtime/registry.rs` | **Deleted** after commits A + B |
| `RuntimeHost` façade | `crates/fireline-conductor/src/runtime/mod.rs` | **Thinned to near-zero or deleted** (§6.5 option (b)) |

The §5.1 façade-vs-collapse question tilts toward **collapse**: once the registry and the state machine are both gone, the only thing `RuntimeHost` wraps is `RuntimeLauncher`, and a one-field wrapper is not worth a type.

---

## 7. Host / Sandbox / Orchestrator reframe (post-stress-test)

> **Status:** second directional overlay. §7 supersedes parts of §1-§6 at a layer *above* them. §1-§6 remain correct as descriptions of what's **inside** the fireline Host implementation; §7 explains what the fireline Host *is*.
>
> **Companion:** [`./client-primitives.md`](./client-primitives.md) is the authoritative TS-side formalization of the reframe. It defines `Host`, `Sandbox`, `Orchestrator`, the combinator algebra, and the module layout. §7 here is the Rust-workspace translation and the reconciliation with §1-§6.

### 7.1 The trigger

A conversational stress test on **2026-04-11** asked *"what if the Host was the Claude Agent SDK v2 preview's `query({ resume: sessionId })` instead of fireline's control plane?"*. That exercise revealed two things:

1. **The proposal's whole §1-§6 is talking about the internals of one specific `Host` implementation** — the fireline-native one (conductor + control plane + stream projection). Everything named here (`RuntimeLauncher`, `RuntimeSpecJournal`, `RuntimeLifecycle`, the stream-as-truth sequence) is internal to that impl.
2. **There is a second primitive adjacent to Host that this proposal was quietly conflating with it** — `Sandbox` in the Anthropic managed-agent sense (the tool-execution environment used *inside* a running session for bash / code / browser / fs calls), which is distinct from "the thing that runs an agent session."

`client-primitives.md` formalizes the three primitives in TypeScript. This section maps them onto the Rust workspace and updates §3/§4/§6 in light of the reframe.

### 7.2 The three primitives, named

From `client-primitives.md`:

- **`Host`** — provisions a runtime (a place where an agent process can run) and owns its lifecycle (`provision`, `wake`, `status`, `stop`). Sessions live inside a provisioned runtime on the ACP data plane, minted by `session/new` — they are not a Host-primitive verb. Implementations: the fireline control plane + conductor (everything §1-§6 describes) for the native satisfier; future hosted-API satisfiers as they come. (A Claude Agent SDK v2 `host-claude` satisfier was attempted and deleted in commit `37db346`; the design lessons are preserved in [`../explorations/claude-agent-sdk-v2-findings.md`](../explorations/claude-agent-sdk-v2-findings.md) and in `client-primitives.md` §"Stress test, in retrospect".)
- **`Sandbox`** — runs a *single tool call* in isolation inside a running session. `provision(resources) → execute(call) → release()`. Anthropic's framing: "any executor called many times as a tool." Implementations: **microsandbox** (microVM-per-sandbox), local subprocess, Docker, Bun.Sandbox, e2b, Daytona.
- **`Orchestrator`** — substrate-agnostic wake loop over a `SessionRegistry`. Only touches `host.wake(handle)`. Indifferent to which `Host` or `Sandbox` is in use.

Full TS signatures and contracts in `client-primitives.md` Modules 2, 3, and the orchestrator section. The Rust workspace will grow parallel traits (named TBD in §7.6) that satisfy the same algebra.

### 7.3 What this reframes about §1-§6

The three concerns §3 extracts and §6.2 further dissolves are **internals of the fireline `Host` impl**, not public substrate primitives. Re-reading §1-§6 under this lens:

- **§1 Current state** — still correct. Describes what the fireline `Host` impl is doing inside `RuntimeHost::create/stop/delete/register/heartbeat`.
- **§2 Pressure points** — still correct. The `pending_runtime_specs` cache and the §2-era race are pathologies of one specific `Host` impl's internal bookkeeping.
- **§3 Proposed split** — §3a `RuntimeLauncher` still survives as an internal. §3b `RuntimeSpecJournal` still dissolves (per §6.2). §3c `RuntimeLifecycle` still shrinks. None of these are ever part of the fireline-as-substrate public API — they live *behind* the `Host` trait, not on it.
- **§4 Transition plan** — its commits still happen. They are now understood as "tidy up the internals of the fireline `Host` impl," not as "architect the general split." The general split is `client-primitives.md`.
- **§6 Stream-as-truth** — still correct. Stream-as-truth is the state substrate the fireline `Host` impl uses. Other `Host` impls have their own state stores (Claude's session state lives server-side in Anthropic's infrastructure; there is no stream for us to read). Stream-as-truth is not a requirement of the `Host` primitive — it's a property of fireline's specific satisfier.
- **`RuntimeProvider` / `ManagedRuntime` / `LocalProvider` / `DockerProvider`** — these are the execution backends the fireline Host uses to run **its own runtime subprocess** (the `fireline` binary, hosting the conductor + the agent child process). They are **not** instances of the new `Sandbox` primitive, despite the naming collision. They're internal to fireline-the-Host; `Sandbox` is a different primitive at a different layer.

### 7.4 Microsandbox, categorized

The microsandbox investigation earlier in this session was evaluating it **as a `RuntimeProvider` replacement** — boot a microVM, run the `fireline` binary inside it, reach the control plane back on the host, expose the ACP WebSocket via reverse port publish, etc. That framing produced a real host-reachability investigation (the LAN-IP discovery, the `NetworkPolicy::allow_all()` requirement, the whole "guest `127.0.0.1` is the guest's own `lo0`" clarification) — all of which was correct, but was solving the wrong problem because microsandbox was in the wrong slot.

**Under §7, microsandbox is a `Sandbox` impl, not a `RuntimeProvider` replacement.** The shape:

```rust
// Sketch — actual trait lives in the substrate crate (§7.6 open question).
impl Sandbox for MicrosandboxSandbox {
    async fn provision(&self, resources: &[ResourceRef]) -> Result<SandboxHandle> {
        // Sandbox::builder(...).image("alpine").network(|n| n.policy(allow_all()))
        //     .replace().create_detached().await
        // Then sandbox.fs().copy_from_host(...) for each resource.
    }
    async fn execute(&self, h: &SandboxHandle, call: ToolCall) -> Result<JsonValue> {
        // sandbox.exec_stream(call.cmd, call.args).await — collect stdout/stderr/exit.
    }
    async fn release(self, h: SandboxHandle) -> Result<()> {
        // sandbox.stop_and_wait().await?; sandbox.remove_persisted().await
    }
}
```

Under this shape, **the entire host-reachability investigation becomes moot**. The microsandbox handle is owned by the `Host` process (fireline-native or Claude-backed), not by a fireline runtime inside the VM. The VM never needs to call back to anything on the host: tool calls enter via `execute()`, tool output leaves via the return value. The only networking a `MicrosandboxSandbox` needs is **outbound egress for tool calls themselves** (a `curl` inside `bash`, a `pip install` inside `code`), which `allow_all` already provides cleanly.

The spike findings from earlier this session (373 ms cold boot, 5 ms clean stop, `copy_from_host` works, `exec_stream` works, `allow_all` opens egress) **all still apply**; they're exactly the right facts about microsandbox for this role. The integration is smaller, not larger: **no LAN-IP discovery helper, no `--host 0.0.0.0` flag-flipping on the control plane, no env-var gymnastics.** Estimate drops from half-day to **~2-3 hours** for the happy-path `MicrosandboxSandbox` impl once the `Sandbox` trait exists to implement against.

### 7.5 Transition plan under the reframe (supersedes §6.4 *at a layer above*)

§6.4's commit sequence still happens — it describes fireline-Host internal cleanup and is correct as such. A sibling sequence sits above it:

**Tier A — `client-primitives.md` Tier 1-3 (TS substrate).** Land `@fireline/client/core`, `@fireline/client/host`, `@fireline/client/orchestration`, `@fireline/client/host-fireline`. Full scope in that doc. ~2.5 days per its build order.

**Tier B — §6.4 stream-as-truth cleanup (Rust, fireline-Host internal).** Independent of Tier A; can run in parallel. Scoped entirely to `crates/fireline-conductor/src/runtime/` and adjacent control-plane code. See §6.4 for the commit-by-commit sequence.

**Tier C — Introduce Rust `Host` + `Sandbox` traits.** Blocks on B (to avoid moving targets) but not on A. Names the primitives in Rust, wraps existing code behind the new traits. Mostly renaming and regrouping; zero semantic change. ~1 day.

**Tier D — `MicrosandboxSandbox: Sandbox`.** Blocks on C. ~2-3 hours for a happy-path impl plus a single green test under `tests/sandbox_microsandbox.rs` (or equivalent). First non-trivial consumer of the new `Sandbox` trait.

**~~Tier E — (optional, demo-facing) `host-claude` TS satisfier~~** (attempted, deleted in commit `37db346`). The satisfier was written and then removed along with its Tier 6 slot in `client-primitives.md`; the design lessons it surfaced informed the Host/Sandbox cleave and the `Host.provision` rename and are preserved in [`../explorations/claude-agent-sdk-v2-findings.md`](../explorations/claude-agent-sdk-v2-findings.md) + `client-primitives.md` §"Stress test, in retrospect". A future second-satisfier (hosted-API, Docker-managed, etc.) would reinstate the "two Hosts + one Sandbox" demo framing, but is not on the critical path.

Tiers A and B can run in parallel. C blocks D. E is parallel to C and D once A is in. The critical path for the **microsandbox demo story** specifically is B → C → D → E: ~3-4 days end to end if one lane at a time, faster with lane parallelism.

### 7.6 New open questions

Replacing / extending §5, specifically at the Rust-workspace layer:

1. **Where does the Rust `Sandbox` trait live?** Options: grow `fireline-conductor` to carry it (simplest, but couples substrate to runtime wiring); add a new `fireline-substrate` crate at the bottom of the workspace that both `fireline-conductor` and downstream `Sandbox` impls depend on (cleanest, but a new crate). Leaning toward **new `fireline-substrate` crate** to mirror the TS split between `@fireline/client/core` (primitives) and `@fireline/client/host-fireline` (satisfier).
2. **Does the Rust `Host` trait mirror the TS `Host` contract method-for-method?** The TS has `provision`, `wake`, `status`, `stop` (post-`37db346` rename from the earlier `createSession / stopSession`; see `client-primitives.md` Module 2). The Rust analogue could be identical, or it could stay Rust-native (richer types). Symmetry is cheap and makes the cross-language story legible; arbitrary divergence is a future cost. Leaning symmetric unless a specific Rust-only concern surfaces.
3. **Does `ResourceRef` hoist out of `fireline-conductor`?** Today it lives in `crates/fireline-conductor/src/runtime/provider.rs`. Under §7, both `Host` and `Sandbox` consume `ResourceRef` (Host for fireline-runtime mounts, Sandbox for tool-call workspaces). Moving it to the substrate crate makes it a shared primitive, per `client-primitives.md`'s treatment.
4. **What is a `Sandbox` instance's lifespan within a session?** Three options with materially different cost profiles: **per-call** (create/release on every tool use — safest isolation, 373 ms × N calls is expensive even at microsandbox's cold-boot), **per-turn** (one sandbox per prompt turn, reused across N tool calls in that turn — good middle ground), **per-session** (one sandbox for the whole session — lowest latency, some cross-turn state bleed). The `Sandbox` trait itself is indifferent; the fireline `Host` has to choose a policy. Microsandbox's `.max_duration()` and `.idle_timeout()` surfaced in the earlier doc review make **per-turn** ergonomic.
5. **Which fireline-components map to `Combinator`s vs. to `Sandbox` execution?** Per `client-primitives.md`'s seven-combinator algebra, components like `approval_gate`, `budget`, `context_injection`, `audit`, `peer_mcp` decompose into `suspend` / `filter` / `map_effect` / `observe` / `fanout` combinators — they are *not* Sandbox concerns. `fs_backend` is more ambiguous: file read/write from the agent's perspective may route through either a `Sandbox` (if it's workspace access) or a combinator (if it's projection into the session log). This wants a follow-up pass aligned with `managed-agents-mapping.md`.

### 7.7 What becomes redundant in §1-§6 under the reframe

Nothing is deleted, but several framings soften:

- **§3's public-API framing of `RuntimeLauncher`/`RuntimeLifecycle`** — these are never exported. They live in `crates/fireline-conductor/src/runtime/` as internals of the fireline `Host` impl. No external caller sees them.
- **§5.1 façade-vs-collapse** — tilts further toward **collapse**. `RuntimeHost` as a public type has no role once (a) its internals are tidied per §6.4 and (b) it becomes the body of `impl Host for FirelineHost { … }`. The `Host` trait is the only visible surface.
- **§4 Commit 4 ("push the composition boundary up")** — executes *into* the new `impl Host for FirelineHost` block rather than into a fresh `RuntimeHost` struct. Lower blast radius, cleaner cut.
- **Microsandbox-as-`MicrosandboxProvider`-replacing-`DockerProvider`** — fully deleted as a framing. Microsandbox does not touch the `RuntimeProvider` trait. That trait stays an internal of fireline's own subprocess-host strategy.

### 7.8 Closing §5.6: does `RuntimeLifecycle` survive as a named type?

**Resolved: no — it collapses into the fireline `Host` satisfier's internals and does not appear as a named public type.**

§5.6 asked whether `RuntimeLifecycle` should survive as a named Rust type after §6.2's stream-as-truth reframe, where the state machine degenerates into a folder of stream-write helpers plus a projection. §7 resolves it:

- The fireline `Host` impl owns its registration / heartbeat / stop logic as **private methods on the impl block**. These methods are never exposed across the `Host` trait boundary.
- The Rust `Host` trait's public verbs are the TS-side four: `create_session`, `wake`, `status`, `stop_session` (plus Rust-flavored async / `Send + Sync` bounds). The trait has no `register` or `heartbeat` method — those are a satisfier-internal concern of the fireline implementation specifically, because push-mode registration is a property of fireline's subprocess/container strategy, not a property of every Host (Claude's Host does none of this).
- The stream-write helpers (`emit_runtime_endpoints_persisted`, `emit_runtime_spec_persisted`, etc.) and the `RuntimeIndex` projection continue to exist and are still what §6.2 §3c described. They just don't live inside a type named `RuntimeLifecycle`. They live as free functions in `fireline-conductor::trace` (already where they live today) and as an `impl StateProjection for RuntimeIndex` block. §5.6 option **(b) — drop the name** — wins.

This also answers the related §7.3 concern about `RuntimeProvider` / `ManagedRuntime` / `LocalProvider` / `DockerProvider`: they are also fireline-Host internals, kept under their existing names inside `crates/fireline-conductor/src/runtime/`, invisible through the `Host` trait. They are the execution backend of **fireline's** Host satisfier and do not propagate to other `Host` implementations.

### 7.9 Decision record

`client-primitives.md` is authoritative for the TS-side formalization and for the public substrate layout. This §7 is authoritative for the Rust-workspace manifestation and for reconciling §1-§6 with the reframe. When they disagree on the primitive layer, **`client-primitives.md` wins**; when they disagree on Rust internals, **§7 (and its §6 / §4 transitively) wins**. Both docs should be read together.

If gnijor or workspace:4 signs off on §7, the next concrete proposal artifact would be a one-line update to `client-primitives.md`'s "Related" block to cross-reference §7 here, and the start of Tier A / Tier B lane work per §7.5.
