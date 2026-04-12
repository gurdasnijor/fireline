# fireline-host cleanup execution plan

> **Input:** [`./fireline-host-audit.md`](./fireline-host-audit.md) (commit `8613304`) — the architecture audit that identified the cleanup surface.
> **Purpose:** turn the audit's 6 recommended follow-up lanes and 3 open questions into an ordered, dispatchable execution plan. As soon as workspace:13 finishes the crate restructure (phases 9i–9k), this plan is the ready-to-fire sequence.
> **Doc type:** design-time execution plan. Markdown only. Does not touch code.
> **Related:**
> - [`./runtime-host-split.md`](./runtime-host-split.md) §7 — Host/Sandbox/Orchestrator taxonomy.
> - [`./client-primitives.md`](./client-primitives.md) — canonical client-surface design; `Host.provision` verb (post-`37db346` rename).
> - [`./deployment-and-remote-handoff.md`](./deployment-and-remote-handoff.md) — "one binary per Host, scale horizontally" deployment posture.
> - [`./crate-restructure-manifest.md`](./crate-restructure-manifest.md) — phase status table; workspace:13 execution status.
> - [`../handoff-2026-04-11-stream-as-truth-and-runtime-abstraction.md`](../handoff-2026-04-11-stream-as-truth-and-runtime-abstraction.md) — stream-as-truth phase sequence; §"Current in-flight work".

## Provisional answers to the audit's three open questions

**Every answer here is provisional — user to confirm.** I grounded each in the existing architecture proposals; where the answer is a judgement call, I flagged it.

### Q1. Multi-runtime child provisioning on a single Host, or direct-host-only posture plus external durable streams?

**Provisional answer: direct-host posture is the target; `register` / `heartbeat` are transitional and go away with phase C5.**

Rationale:

- [`./deployment-and-remote-handoff.md`](./deployment-and-remote-handoff.md) frames the target as "one binary per Host, scale horizontally" — that is structurally incompatible with a Host-internal control plane managing child runtimes. A Host either *is* a runtime or it *provisions* exactly one runtime in-process; any multi-child provisioning belongs above the Host, at an orchestrator layer that sits on top of multiple Host satisfiers.
- [`./runtime-host-split.md`](./runtime-host-split.md) §7.2 names `Host.provision` as the verb that "hands out a runtime." §7.3 is explicit that the provisioner's runtime-level internals (`RuntimeProvider` / `ManagedRuntime` / `LocalProvider`) are fireline-Host satisfier internals, not instances of the primitive itself. Nothing in §7 argues for Host-owned multi-runtime lifecycle as a target shape.
- The stream-as-truth handoff explicitly names *"delete `RuntimeRegistry` + `HeartbeatTracker` + stale scanner once the read path flips"* (§"Deferred Steps 2, 3, 4") — which implies the entire "Host holds a registry of in-process children" model is slated for deletion, not expansion.

Implication: phases below treat `register` / `heartbeat` as load-bearing **only until C5 lands**. After C5, they are deleted along with the rest of the multi-runtime bookkeeping.

**Flag:** if workspace:13's phase 9i–9k intentionally preserves a multi-child-per-host pattern (e.g. to support a Host→worker-pool shape for Sandbox satisfiers), this answer is wrong and C3/C5 need a different shape. Confirm before firing C5.

### Q2. Peer discovery — stay on HTTP-poll, or move to stream-backed / tool-native?

**Provisional answer: move to stream-backed peer discovery as part of C5. Until then, keep the HTTP adapter but relocate per C4.**

Rationale:

- The stream-as-truth handoff is clear: *"Heartbeats become optional liveness hints, not source of truth. Control plane is a stateless reader that materializes a `RuntimeIndex` projection from the stream."* Peer discovery is the same-shape problem as liveness — "which runtimes exist, what are their endpoints" — and the same-shape answer applies: derive from the durable stream's `runtime_endpoints` projection, not from an HTTP poll against `GET /v1/runtimes`.
- The audit (Q2 / §Category A) correctly places `ControlPlanePeerRegistry` in the Tools primitive, not Host. A stream-backed `PeerRegistry` satisfier (call it `StreamPeerRegistry`) belongs in `fireline-tools` reading from the shared stream's `runtime_endpoints` rows.
- The HTTP adapter can survive C4's relocation (renamed to `HttpRuntimePeerRegistry` and moved to `fireline-tools`) as an interim bridge for any caller that can't subscribe to a shared stream yet. C5 deletes it along with the rest of the HTTP runtime-list surface.

**Flag:** the interim relocation in C4 is cheap if it buys us anything, wasteful if C5 lands promptly after. See C4's "Re-scope consideration" note below.

### Q3. Browser/workspace file-helper REST API — revive, or deprecate?

**Provisional answer: deprecate. Do not revive the REST shape. The Resources primitive + future MCP path owns this.**

Rationale:

- [`./client-primitives.md`](./client-primitives.md) Module 1 defines `ResourceRef` as a first-class serializable type, and [`./runtime-host-split.md`](./runtime-host-split.md) §7 places resource-mounting concerns in `fireline-resources` under `ResourceMounter` + `FsBackendComponent`, not under a REST helper API.
- The audit (§Category A, §Category C) confirms both `crates/fireline-host/src/connections.rs` and `crates/fireline-resources/src/routes_files.rs` are dead TODO stubs with zero production callers. The "lookup file" mechanism they sketched never landed and has been superseded by the combinator / MCP path.
- ACP's own `fs/read_text_file` and `fs/write_text_file` protocol methods flow through the conductor proxy chain as effects — which is the right boundary for file operations, not a parallel REST surface.

**Pre-C1 investigation flag:** the dispatch explicitly asks to confirm the browser harness isn't currently calling either dead surface before the delete. Based on my earlier reads of `packages/browser-harness/src/app.tsx` and `dev-server.mjs` (Tier 5 rewire, commit `52c31af`), the browser-harness Tier 5 path only calls `/api/agents`, `/api/resolve?agentId=...`, and `/cp/v1/runtimes*`. **No `/fs/*` or connection-lookup REST calls.** The dead-surface deletion in C1 is safe. If a separate dispatch wants a belt-and-suspenders confirmation with a `grep -r` sweep, happy to do that; I did not execute it as part of this plan.

---

## Phase overview

| Phase | Title | Audit lane | Risk | Commits | Depends on | Conflicts-with-w13-9i |
|---|---|---|---|---|---|---|
| **C1** | Delete dead surfaces | lane 1 + Category C | **low** | 1–2 | — | **maybe** |
| **C2** | Collapse direct-host path | lane 2 | **medium** | 2–3 | C1 (safer with dead code gone first) | **yes** |
| **C3** | Delete `/v1/auth/runtime-token` + `auth.rs` prep | lane 4 (part) | **low-medium** | 1 | C1 | **maybe** |
| **C4** | Peer registry collapse + stream-backed `StreamDeploymentPeerRegistry` | lane 5 (rescoped) | **medium** | 3–4 | C1 + `cross-host-discovery.md` landed | **maybe** |
| **C4b** | Resource discovery collapse + `StreamResourceMounter` | new | **medium** | 2–3 | C1 + `resource-discovery.md` landed | **maybe** |
| **CB** | Naming drift — `create_runtime` → `provision_runtime` etc. | Category B | **low** | 1–2 | interleaves any time after C2 | **maybe** |
| **C5** | Stream-as-truth flip + delete `heartbeat.rs` + `HeartbeatTracker` + stale scanner + `/register` + `/heartbeat` + `auth.rs` residue | lane 3 (+ completes lane 4) | **high** | 4–6 (multi-step) | C1, C2, C3, C4 | **yes** |
| **C6** | Rewrite or inline `bootstrap.rs` | lane 6 | **high** | 1–2 | C2, C5 | **yes** |

**Total estimated commits:** 15–22 across all phases (C4 and C4b added ~4–7 commits to the original estimate). Plan to hold a cleanup PR branch for each phase; do not mix phases in one commit.

## Ordering rationale

The dispatch's suggested ordering (C1 → C2 → C3 → C4 → CB interleave → C5 → C6) is preserved. Key sequencing constraints:

- **C1 before C2**: deleting the dead `build.rs` / `transports/*.rs` / `connections.rs` / `routes_files.rs` shrinks the surface C2 has to reason about. Not strictly required but removes noise.
- **C2 before C5**: C5 deletes the multi-runtime bookkeeping (`RuntimeRegistry`, heartbeat). The direct-host path currently reuses that bookkeeping via `BootstrapRuntimeLauncher` + `register()`. If C5 lands before C2, direct-host mode breaks mid-sequence. **C2 must land first.**
- **C3 before C5**: C5 deletes `auth.rs` residue along with `/register` + `/heartbeat`. Deleting the public `/v1/auth/runtime-token` route first (in C3) is safe because built-in launchers never call it. The remaining middleware-on-register/heartbeat is deleted with those routes in C5.
- **C4 before C5** (optional): if C4 relocates the registry to `fireline-tools` as an interim HTTP adapter, C5 then deletes that interim adapter when stream-backed discovery lands. If C4 is skipped (treated as "wait for C5 and delete directly"), C5 deletes `control_plane_peer_registry.rs` from its original location. Either sequencing works; see C4's re-scope note.
- **C5 before C6**: C6 rewrites `bootstrap.rs` around a required external durable-streams URL. That rewrite is only correct once the read path has flipped (C5's stream-as-truth transition). Doing C6 before C5 would embed assumptions that C5 invalidates.
- **CB can interleave anywhere after C2**: it's a pure rename. Before C2, some of the target symbols (`create_runtime`, `create`) are on files C2 is about to delete or substantially rewrite, so there's no point renaming them. After C2, the remaining `create` / `create_runtime` mentions are on files that survive C5 (router.rs, fireline-sandbox/lib.rs) and benefit from the rename.

---

## Phase C1 — Delete dead surfaces

**Scope.** Pure file deletions + tiny cleanups. Zero semantic changes. Everything listed is proven dead by the audit (Category C + Category A dead entries).

**Exact files touched** (from audit §Category C and §Category A):

| File | Action | Audit reference |
|---|---|---|
| `crates/fireline-host/src/connections.rs` | delete (dead TODO stub, no production callers) | `:1-37`, audit:199 |
| `crates/fireline-resources/src/routes_files.rs` | delete (dead TODO stub) | `:1-30`, audit:200 |
| `crates/fireline-runtime/src/connections.rs` | delete (compatibility shim over deleted stub, `#[allow(unused_imports)]`) | `:1-2`, audit:205 |
| `crates/fireline-host/src/build.rs` | delete (dead — live ACP route is `crates/fireline-harness/src/routes_acp.rs:127-150`) | `:1-62`, audit:201 |
| `crates/fireline-host/src/transports/websocket.rs` | delete (dead duplicate of `crates/fireline-harness/src/routes_acp.rs:152-188`) | `:1-58`, audit:202 |
| `crates/fireline-host/src/transports/duplex.rs` | move to `fireline-harness` test helpers OR delete | `:1-26`, audit:180 |
| `crates/fireline-host/src/transports/mod.rs` | delete (the module contains only the two files above) | follows from above |
| `crates/fireline-host/src/router.rs:254-260` | remove unused `_scope` parameter on `issue_runtime_token()` — trivial internal cleanup | audit:204 |
| `crates/fireline-sandbox/src/provider.rs:11-19` | remove unused `RuntimeLaunch.status` field | audit:206 |
| Any `mod connections;` / `mod build;` / `mod transports;` / `pub use transports::*` in the affected `lib.rs` files | remove the now-dangling module declarations | follows mechanically |
| Any `use fireline_host::transports::websocket::*` or similar in workspace integration tests | leave the test surface in place ONLY if it still compiles against `fireline-harness`'s equivalent; otherwise switch the test's import path to the harness copy | follows from duplication cleanup |

**Done-when checklist:**

- [ ] `grep -rn 'mod connections' crates/fireline-host/src` → empty
- [ ] `grep -rn 'connections::LookupFile\|pub use connections' crates/ src/ tests/` → empty
- [ ] `grep -rn 'fireline_host::build\|fireline_host::transports' crates/ src/ tests/` → empty (or all matches point at the surviving `fireline-harness` equivalents)
- [ ] `grep -rn 'routes_files' crates/fireline-resources/src` → empty
- [ ] `grep -rn '_scope' crates/fireline-host/src/router.rs` → empty
- [ ] `cargo check --workspace` green
- [ ] The managed-agent test suite still compiles (no `--no-run` failures)

**Risk: LOW.** Every file in this list is confirmed dead by the audit's file:line citations. No behavior changes. The only risk is that workspace:13's phase 9i–9k is concurrently moving one of these files between crates — in which case C1 conflicts mechanically but not semantically.

**Conflicts-with-w13-9i: maybe.** `crates/fireline-runtime/src/connections.rs` and the `fireline-host/src/transports/*` files are exactly the kind of dead-shim surfaces workspace:13 might be pruning as part of its dissolved-crate cleanup. **Best execution order:** wait for workspace:13 to signal "phase 9i done, check the tree," then run C1 against whatever's left. If workspace:13 already deleted some of these files, C1 shrinks accordingly.

**Estimated commits:** 1 single "Delete dead host + resources surfaces" commit covering everything in the table above. If `transports/duplex.rs` gets *moved* to `fireline-harness` rather than deleted, split into 2 commits (one move, one delete-the-rest).

---

## Phase C2 — Collapse direct-host path

**Scope.** Delete `BootstrapRuntimeLauncher` and the `runtime_host.rs` wrapper. Direct-host mode in `src/main.rs` calls `bootstrap::start` directly instead of tunneling through `RuntimeHost::create → BootstrapRuntimeLauncher → register()`.

**Exact files touched** (from audit Q4 + §Category D):

| File | Action | Audit reference |
|---|---|---|
| `crates/fireline-host/src/runtime_host.rs:20-25, 35-55` | delete the `BootstrapRuntimeLauncher`-backed direct-host wrapper entirely | audit:97–99, 120 |
| `crates/fireline-host/src/runtime_provider.rs:11-61` | delete `BootstrapRuntimeLauncher` | audit:91, 118–120 |
| `crates/fireline-runtime/src/runtime_host.rs` | delete compatibility shim (if present post-9i) | audit:214 |
| `crates/fireline-runtime/src/runtime_provider.rs` | delete compatibility shim (if present post-9i) | audit:214 |
| `src/main.rs:138-166` | rewrite direct-host mode to call `bootstrap::start(BootstrapConfig {..})` directly, skipping `RuntimeHost::create` and the `register()` state-machine reuse | audit:96, 102–105 |
| `crates/fireline-host/src/lib.rs` | remove `pub mod runtime_host` / `pub mod runtime_provider` declarations (or narrow to `ChildProcessRuntimeLauncher` only) | follows |
| `crates/fireline-host/src/local_provider.rs` | **no changes in C2** — `ChildProcessRuntimeLauncher` stays (it is the only legitimately earning launcher per the audit) | audit:121 |

**Done-when checklist:**

- [ ] `grep -rn 'BootstrapRuntimeLauncher' crates/ src/` → empty
- [ ] `grep -rn 'RuntimeHost::create' src/main.rs` → empty (direct-host mode no longer calls `RuntimeHost::create`)
- [ ] `cargo check --workspace` green
- [ ] `cargo test --workspace --no-run` green
- [ ] Direct-host mode integration test (whichever test covers `fireline` in non-push-mode) still passes
- [ ] Managed-agent suite still green

**Risk: MEDIUM.** The direct-host path is live and exercised by tests. The rewrite of `src/main.rs` is small in lines but easy to get subtly wrong — the `bootstrap::start(BootstrapConfig {..})` call signature needs to match what `BootstrapRuntimeLauncher::start_local_runtime` was passing through. Extract a careful line-by-line diff before committing, and run the managed-agent harness suite against the commit before pushing.

**Conflicts-with-w13-9i: YES.** Workspace:13 is currently moving files around in exactly this neighborhood (`fireline-host`, `fireline-runtime`). Do **not** start C2 until workspace:13 announces 9i/9j/9k are landed and `origin/main` is stable.

**Estimated commits:** 2–3.
1. Delete the direct-host wrappers (`runtime_host.rs` in `fireline-host`, both compatibility shims in `fireline-runtime`).
2. Rewrite `src/main.rs` direct-host mode to call `bootstrap::start` directly.
3. (Optional) Delete `runtime_provider.rs` or narrow to only the `ChildProcessRuntimeLauncher` surface if that trait is still useful for C5's child-runtime survival period.

---

## Phase C3 — Delete `/v1/auth/runtime-token` public route

**Scope.** Delete the public token-issuance HTTP route. The bearer-token middleware on `/register` and `/heartbeat` stays temporarily (deleted by C5).

**Exact files touched** (from audit Q1 + §Category C):

| File | Action | Audit reference |
|---|---|---|
| `crates/fireline-host/src/router.rs:34, 101-119` | delete the `POST /v1/auth/runtime-token` route handler and its registration | audit:27, 203 |
| `crates/fireline-host/src/auth.rs:27-75` | leave `RuntimeTokenStore` + `require_runtime_bearer` middleware in place for now — C5 deletes them | audit:19 |
| `crates/fireline-host/src/router.rs:254-260` | already covered by C1 (unused `_scope` cleanup) | audit:204 |
| Tests under `tests/` that call `/v1/auth/runtime-token` | delete those test cases (they are not exercising any surface used by built-in launchers) | audit:27 |

**Done-when checklist:**

- [ ] `grep -rn '/v1/auth/runtime-token' crates/ src/ tests/` → empty
- [ ] `grep -rn 'issue_runtime_token' crates/fireline-host/src/router.rs` → empty
- [ ] `cargo check --workspace` green
- [ ] The `register` / `heartbeat` middleware still works — built-in launchers inject tokens directly from `RuntimeTokenStore` at launch time (audit:25), so no-op for them
- [ ] Managed-agent suite green

**Risk: LOW-MEDIUM.** The route is only exercised by tests and manual clients per the audit. The risk is a test that exercises it as a smoke check but needs a replacement; in that case, inline a launch-scoped secret directly into the test fixture rather than reinstating the route.

**Conflicts-with-w13-9i: maybe.** `router.rs` is a central file workspace:13 may touch during crate reorganization.

**Estimated commits:** 1.

---

## Phase C4 — Peer registry collapse + stream-backed replacement

> **Blocked by:** [`docs/proposals/cross-host-discovery.md`](./cross-host-discovery.md) landed on `origin/main` AND user confirmation that `LocalPeerDirectory` is deleted without a dev fallback. (User already confirmed: *"no fallback — it's an architecture violation."*)

**Scope.** Delete both `ControlPlanePeerRegistry` and `LocalPeerDirectory`. Replace with a new `StreamDeploymentPeerRegistry` backed by a durable-stream projection of the `hosts:tenant-<id>` stream. This is no longer a mechanical relocation — it's a net-new implementation with a net-deletion of two legacy peer-discovery paths.

**Exact files touched:**

| File | Action |
|---|---|
| `crates/fireline-tools/src/peer/directory.rs::LocalPeerDirectory` | **delete** — the TOML-on-disk peer registry is an architecture violation under the cross-host-discovery proposal; no fallback, no dev mode |
| `crates/fireline-host/src/control_plane_peer_registry.rs::ControlPlanePeerRegistry` | **delete** — the HTTP-backed `GET /v1/runtimes` adapter is replaced by the stream-backed registry |
| `crates/fireline-tools/src/peer/stream.rs::StreamDeploymentPeerRegistry` | **NEW** — implements `PeerRegistry` trait, backed by durable-stream projection over the `hosts:tenant-<id>` stream. Shape specified in [`./cross-host-discovery.md`](./cross-host-discovery.md). |
| `crates/fireline-tools/src/peer/mod.rs` | `pub mod stream;` replaces `pub mod directory;` (or both coexist for one interim commit if staged that way) |
| `crates/fireline-host/src/bootstrap.rs` | swap peer registry wiring from `LocalPeerDirectory` / `ControlPlanePeerRegistry` to the new stream-backed impl |
| `crates/fireline-host/src/lib.rs` | remove `pub mod control_plane_peer_registry` declaration |
| `crates/fireline-runtime/src/control_plane_peer_registry.rs` | delete compatibility shim (if present post-restructure) |
| `LocalPeerDirectory::default_path()` callers in `src/main.rs`, `src/bin/agents.rs`, `tests/` | delete the caller sites + any tests that construct a `LocalPeerDirectory` |
| `--peer-directory-path` CLI flag in `src/main.rs` and `fireline-control-plane` | delete — there is no peer-directory file anymore |

**Done-when checklist:**

- [ ] `grep -rn 'LocalPeerDirectory\|local_peer_directory\|peer_directory_path\|PeerDirectoryPath\|peers\.toml' crates/ src/ tests/` → empty
- [ ] `grep -rn 'ControlPlanePeerRegistry\|control_plane_peer_registry' crates/ src/` → empty
- [ ] `grep -rn 'StreamDeploymentPeerRegistry' crates/fireline-tools/src/` → at least one definition match with a `PeerRegistry` trait impl
- [ ] `cargo check --workspace` green
- [ ] Managed-agent suite green — push-mode tests that exercised the old peer discovery path are either updated to use the stream-backed registry or deleted as no-longer-meaningful
- [ ] Bootstrap in push mode constructs `StreamDeploymentPeerRegistry` and passes it to `PeerComponent`; `PeerComponent::list_peers()` returns entries derived from the durable stream, not from an HTTP poll or a TOML file

**Risk: MEDIUM.** This is no longer just a move — it's a net-new `PeerRegistry` implementation that reads from a durable stream. The stream schema depends on `cross-host-discovery.md` which hasn't landed yet. If the proposal's stream shape changes, the impl changes. The risk is bounded by the `PeerRegistry` trait contract: as long as the new satisfier returns `Vec<Peer>` from `list_peers()` and `Option<Peer>` from `lookup_peer()`, the rest of the system doesn't care where the data came from.

**Conflicts-with-w13-9i: maybe.** `crates/fireline-tools/src/peer/` is a directory workspace:13 moved during the restructure; confirm the current layout before starting.

**Estimated commits:** 3–4.
1. Add `StreamDeploymentPeerRegistry` scaffolding in `crates/fireline-tools/src/peer/stream.rs` (empty `PeerRegistry` trait impl returning `Ok(vec![])` / `Ok(None)`)
2. Implement the stream projection — subscribe to `hosts:tenant-<id>`, materialize `Peer` entries from `runtime_endpoints` envelopes
3. Swap bootstrap wiring in `crates/fireline-host/src/bootstrap.rs` + delete the `--peer-directory-path` CLI flag
4. Delete `LocalPeerDirectory` + `ControlPlanePeerRegistry` + their callers + all `peers.toml` references

---

## Phase C4b — Resource discovery collapse

> **Blocked by:** [`docs/proposals/resource-discovery.md`](./resource-discovery.md) landed on `origin/main`.

**Scope.** Delete the assumption that resources are "local paths only" and add a `StreamResourceRegistry` in `fireline-resources` (or `fireline-tools` — the resource-discovery proposal decides placement). Wire into `FsBackendComponent` so that resource lookup crosses Host boundaries via the `resources:tenant-<id>` stream.

This phase is a sibling of C4: C4 collapses peer discovery from file/HTTP to stream-backed; C4b collapses resource discovery the same way. Both share the same durable-streams-as-discovery-plane insight.

**Exact files touched** (specifics depend on `resource-discovery.md`):

| File | Action |
|---|---|
| `crates/fireline-resources/src/mounter.rs` or wherever `LocalPathMounter` is the sole mounter | add a `StreamResourceMounter` sibling that resolves `DurableStreamBlob`-backed refs by reading the `resources:tenant-<id>` stream |
| `crates/fireline-resources/src/fs_backend.rs` (or `fireline-host/src/bootstrap.rs`) | wire the new mounter into the resource-resolution chain alongside `LocalPathMounter` |
| Any code that assumes `ResourceRef::LocalPath` is the only variant in production | handle the `DurableStreamBlob` variant or error explicitly |
| `crates/fireline-resources/src/lib.rs` | export the new `StreamResourceMounter` (or `StreamResourceRegistry` — naming per the proposal) |

**Done-when checklist:**

- [ ] A `ResourceRef::DurableStreamBlob { stream, key }` variant can be round-tripped through the mounter chain — `provision` with a `DurableStreamBlob` ref on a Host connected to a shared durable-streams service resolves the blob and mounts it in the sandbox
- [ ] `cargo check --workspace` green
- [ ] At least one integration test demonstrates cross-Host resource resolution: Host A publishes a resource blob, Host B provisions with a `DurableStreamBlob` ref pointing at it, the content arrives in the sandbox

**Risk: MEDIUM.** Net-new implementation that depends on a proposal not yet landed. The risk is bounded by the `ResourceMounter` trait contract: the new mounter only needs to satisfy the same `mount(ResourceRef, runtime_key) → Option<MountedResource>` shape that `LocalPathMounter` already satisfies.

**Conflicts-with-w13-9i: maybe.** `crates/fireline-resources/` was recently created by the restructure; confirm layout before starting.

**Estimated commits:** 2–3. The specifics depend on `resource-discovery.md`; this plan only reserves the phase slot.

**Note:** C4b does NOT delete `LocalPathMounter`. Local-path resources remain valid for single-Host development mode. What C4b does is *add* the stream-backed resource mounter so cross-Host resource resolution works. This is unlike C4 (which *deletes* `LocalPeerDirectory` entirely without a fallback) because `LocalPathMounter` is not an architecture violation — it's a legitimate single-Host case.

---

## Phase CB — Naming drift rename

**Scope.** Pure symbol renames to align with the `Host.provision` vocabulary from [`./client-primitives.md`](./client-primitives.md) §Module 2 (post-`37db346`). Zero semantic changes.

**Exact symbols renamed** (from audit §Category B):

| Current | Target | File:line | Notes |
|---|---|---|---|
| `router.rs::create_runtime` (handler fn) | `provision_runtime` | `crates/fireline-host/src/router.rs` (audit:188) | Handler name only — the HTTP path `POST /v1/runtimes` does **not** change (deployment doc refers to this URL shape). |
| `RuntimeHost::create` | `RuntimeHost::provision` | `crates/fireline-host/src/runtime_host.rs` (audit:189) | Only applies to the post-C2 surviving `RuntimeHost` if any. If C2 deleted the whole wrapper, this row is moot. |
| `fireline-sandbox::lib.rs::create` | `provision` | `crates/fireline-sandbox/src/lib.rs:45-50` (audit:190) | The `fireline-sandbox` primitive — this is the Host-satisfier entrypoint the client-primitives doc calls `Host.provision`. |
| `runtime_provider.rs::start_local_runtime` | **keep as-is** | `crates/fireline-host/src/runtime_provider.rs` (audit:191) | Audit note: "if kept internal, name it around process launch instead." Since C2 deletes this file outright, this row becomes moot. |
| `local_provider.rs::start_local_runtime` | **keep `start_local_runtime` as an internal process-launch name** | `crates/fireline-host/src/local_provider.rs` (audit:192) | Audit note: internal name should describe process launch, not leak a primitive surface. The name already describes process launch; no rename needed. Close this audit row with "keep." |
| `router.rs::register_runtime` | **delete in C5** | `crates/fireline-host/src/router.rs` (audit:193) | Audit: "No Host primitive verb; make it internal stream/update plumbing or delete." Under Q1's provisional answer, delete with C5. |
| `router.rs::heartbeat_runtime` | **delete in C5** | `crates/fireline-host/src/router.rs` (audit:194) | Audit: "No Host primitive verb; delete under stream-as-truth." C5. |
| `control_plane_client.rs::spawn_heartbeat_loop` | **delete in C5** | `crates/fireline-host/src/control_plane_client.rs` (audit:195) | Audit: "No primitive verb; delete with the stale scanner path." C5. |

**Done-when checklist:**

- [ ] `grep -rn 'RuntimeHost::create\b' crates/ src/` → empty (every hit renamed to `provision`)
- [ ] `grep -rn 'create_runtime' crates/fireline-host/src/router.rs` → empty
- [ ] `grep -rn 'fn create\b' crates/fireline-sandbox/src/lib.rs` → empty at the primitive entry
- [ ] `cargo check --workspace` green
- [ ] Managed-agent suite green

**Risk: LOW.** Pure renames. The main risk is missing a caller; use `rust-analyzer` rename or `cargo check --workspace` to catch stragglers. Every surface-visible HTTP path stays the same (per the note about `/v1/runtimes` not changing).

**Conflicts-with-w13-9i: maybe.** The renames touch files workspace:13 is moving between crates.

**Estimated commits:** 1–2. Can be a single commit if all the renames land together, or split into "Rename Host entrypoints to provision" + "Delete `_scope` etc. cleanup" if it's cleaner for review.

**When to run:** anytime after C2 lands (before C2, half the targets are on files that C2 deletes or substantially rewrites). CB can run in parallel with C3/C4 on separate branches if the conflict surface is narrow enough.

---

## Phase C5 — Stream-as-truth flip + delete `heartbeat.rs` + registry liveness

**Scope.** The biggest semantic change. Deletes `HeartbeatTracker`, the stale scanner, the `/register` and `/heartbeat` routes, the `auth.rs` residue, `ControlPlanePeerRegistry` (if not relocated by C4), and flips the control-plane read path to use the stream-derived `RuntimeIndex` instead of the file-backed `RuntimeRegistry`.

**Internal sub-phases** (these serialize; do NOT collapse into one commit):

### C5a — Add shared-state-stream subscription to production control plane (new capability)

The stream-as-truth handoff identified this as the "non-trivial infra" blocker for deleting `RuntimeRegistry`:

- Add a `--shared-state-stream-url` CLI flag (or equivalent config) on the control plane binary
- Spawn a `RuntimeMaterializer` with a `RuntimeIndex` subscribed to that URL at startup
- Pass the same URL to every runtime the control plane spawns as `FIRELINE_EXTERNAL_STATE_STREAM_URL` (already passed when `--shared-stream-base-url` is set via `local_provider.rs:146-151` per the handoff doc)

**Files touched:** `crates/fireline-host/src/control_plane.rs`, `crates/fireline-host/src/local_provider.rs`, possibly `src/main.rs` / `crates/fireline-host/src/bootstrap.rs` (for the control-plane bin's arg parsing and startup).

**Done-when:** control plane binary boots with `--shared-state-stream-url=<url>` and the `RuntimeIndex` preloads without error. Writes still go through `RuntimeRegistry` (this sub-phase is additive only).

### C5b — Flip the read path: `router::list_runtimes` and `get_runtime` read from `RuntimeIndex`

- `crates/fireline-host/src/router.rs:49-99` — `list_runtimes` and `get_runtime` call `RuntimeIndex::endpoints_for` / `list_endpoints` instead of `RuntimeRegistry::get` / `list`
- Writes continue to go to `RuntimeRegistry` as rollback safety until C5c

**Done-when:** `cargo check --workspace` green, agreement test (`tests/runtime_index_agreement.rs`) still passing, control-plane integration tests green.

### C5c — Delete `HeartbeatTracker`, `heartbeat.rs`, stale scanner, `/register`, `/heartbeat`, `auth.rs` residue, registry liveness methods

- `crates/fireline-host/src/heartbeat.rs:5-26` — **delete file**
- `crates/fireline-host/src/control_plane.rs:107-187` — delete stale-scanner spawn + body
- `crates/fireline-host/src/router.rs:121-175` — delete `register_runtime` + `heartbeat_runtime` handler bodies AND their routes
- `crates/fireline-host/src/router.rs:140-175` — delete registry-liveness-writing code in those handlers
- `crates/fireline-host/src/auth.rs:27-75` — **delete file** (middleware it provided is now unused after the `/register` + `/heartbeat` delete above)
- `crates/fireline-host/src/lib.rs` — remove `pub mod heartbeat` and `pub mod auth`
- `crates/fireline-host/src/control_plane_client.rs:86-119` — delete `spawn_heartbeat_loop()`
- `crates/fireline-host/src/runtime_registry.rs` (or wherever `RuntimeRegistry` lives post-9i) — delete liveness methods (`record_heartbeat`, `mark_stale_before`, `get_last_seen_ms`, etc. per the audit's §Category E references)
- If C4 didn't already relocate it: `crates/fireline-host/src/control_plane_peer_registry.rs` — delete entirely (HTTP runtime-list surface is gone; stream-backed discovery takes over)
- **Add:** `StreamPeerRegistry` implementation in `fireline-tools` that reads from the `RuntimeIndex` projection — this is the replacement peer-discovery mechanism the stream-as-truth handoff names
- `tests/` — delete or update any tests that exercise the deleted routes

### C5d — Delete `RuntimeRegistry` file-write path entirely

- `crates/fireline-host/src/runtime_registry.rs` — delete file-backed writes; `create_runtime` / `stop_runtime` / `delete_runtime` now emit `runtime_endpoints` envelopes to the durable stream only
- This is the "write path flip" that mirrors C5b's read-path flip

**Files touched across all C5 sub-phases:** the list is long; see each sub-phase above. The audit's §Category E is the complete target surface.

**Done-when checklist (overall C5):**

- [ ] `grep -rn 'HeartbeatTracker\|heartbeat_loop\|spawn_heartbeat_loop' crates/ src/ tests/` → empty
- [ ] `grep -rn 'auth::require_runtime_bearer\|RuntimeTokenStore' crates/ src/` → empty
- [ ] `grep -rn '/v1/auth/runtime-token\|/register\|/heartbeat' crates/fireline-host/src/router.rs` → empty
- [ ] `grep -rn 'runtime_registry::RuntimeRegistry' crates/fireline-host/src/` → either empty OR only references the stream-backed write path (no liveness methods)
- [ ] `grep -rn 'ControlPlanePeerRegistry' crates/` → empty (replaced by `StreamPeerRegistry` in `fireline-tools`)
- [ ] `cargo check --workspace` green
- [ ] Managed-agent suite green
- [ ] The `runtime_index_agreement` test still passes against the new "reads-from-stream, writes-to-stream" shape
- [ ] Push-mode integration tests still pass (they exercise `/register` + `/heartbeat` today; C5 needs to either update those tests to use the new pattern or delete them as no-longer-meaningful)

**Risk: HIGH.** This is the biggest semantic change in the plan. Four deletion surfaces collapse at once, the read path flips, and the write path flips. Every test that used to exercise the runtime registry's liveness methods needs to be triaged (updated vs deleted).

**Conflicts-with-w13-9i: YES.** Workspace:13's 9i–9k phases touch many of the same files.

**Estimated commits:** 4–6, one per sub-phase above. Do **not** squash.

**Critical pre-C5 confirmation:** validate Q1 with the user before firing C5. If the target is actually multi-child-per-Host (not direct-host-only), register/heartbeat survive and C5c/C5d need a completely different shape.

---

## Phase C6 — Rewrite or inline `bootstrap.rs`

**Scope.** After C5 lands, `bootstrap::start` no longer needs to handle embedded durable streams, optional external-stream URLs, direct-host registration state-machine, or local peer-directory bootstrap. Shrink it to thin runtime-server assembly or inline into `src/main.rs`.

**Exact files touched** (from audit Q6):

| File | Action | Audit reference |
|---|---|---|
| `crates/fireline-host/src/bootstrap.rs` | rewrite: router assembly + `axum::serve` against a **required** external durable-streams URL. Delete `build_stream_router` call, delete `StreamStorageConfig` plumbing, delete `external_state_stream_url: Option`, delete local peer-directory bootstrap, delete the `runtime_spec_persisted` emit (C5 already relocated that to the stream-write path). | audit:143–162, 169–171 |
| `crates/fireline-host/src/bootstrap.rs:3-15` | update/delete the stale doc comment (audit:150 flags it as already inaccurate) | audit:8 |
| `src/main.rs` | optionally inline `bootstrap::start` at the call site if the helper shrinks below the "worth abstracting" threshold | audit:166–170 |
| `crates/fireline-host/src/lib.rs` | narrow or remove `pub mod bootstrap` | follows |

**Done-when checklist:**

- [ ] `bootstrap.rs` is <= ~100 lines (or deleted entirely in favor of inline `src/main.rs` assembly)
- [ ] No references to `build_stream_router`, `StreamStorageConfig`, `external_state_stream_url` in `bootstrap.rs` or `src/main.rs`
- [ ] `cargo check --workspace` green
- [ ] Managed-agent suite green
- [ ] Direct-host mode still boots (from C2, it calls bootstrap directly — if bootstrap is inlined, main.rs does the assembly inline)
- [ ] Child-runtime mode still boots (from `ChildProcessRuntimeLauncher` spawning `fireline` as a subprocess)

**Risk: HIGH.** The bootstrap rewrite is the last semantic consolidation and the most assumption-dense. Every boot path (direct-host, push-mode child, direct-host-with-env-external-stream) has to still produce a working runtime.

**Conflicts-with-w13-9i: YES.**

**Estimated commits:** 1–2. One for the shrink, one for any inline-into-main.rs follow-up if chosen.

---

## Cross-phase dependencies (not all named by the audit)

- **C2 implicitly depends on the `ChildProcessRuntimeLauncher` survival condition.** If C5's "direct-host only" stance (Q1's provisional answer) holds, C2 should also delete `ChildProcessRuntimeLauncher` and the entire `local_provider.rs` file — the multi-runtime launch surface goes away along with register/heartbeat. **If Q1 is confirmed, merge that delete into C2.** If Q1 is wrong and multi-child survives, keep `ChildProcessRuntimeLauncher` in C2 (as it's the legitimately-earning side of the audit Q4 split).
- **C5c's `StreamPeerRegistry` add is not in the audit.** The audit correctly identified that stream-backed peer discovery belongs in `fireline-tools`, but didn't spec a name for the replacement. This plan names it `StreamPeerRegistry` and slots it into `fireline-tools`; that naming is provisional and may change during implementation.
- **CB's renames can silently regress if C2/C5 delete a file mid-sequence that CB already renamed.** Running CB *after* C5 removes this risk entirely, at the cost of running CB against a smaller surface. If CB runs before C5, the CB author needs to double-check that every symbol they rename is on a file that survives C5 (router.rs does, control_plane_client.rs doesn't because `spawn_heartbeat_loop` is deleted, etc.).
- **C6 and C5 are tightly coupled at the bootstrap.rs boundary.** Specifically: C5 deletes the `auth.rs` middleware, which the current `bootstrap.rs` composes into the router. C6 rewrites `bootstrap.rs` to no longer reference that middleware. If C5 lands first without also removing the middleware composition from `bootstrap.rs`, `bootstrap.rs` will fail to compile. Either (a) C5 needs to also patch `bootstrap.rs`'s router-assembly block as part of deleting `auth.rs`, or (b) C5 and C6 need to land as a single PR sweep. **Recommend (b)**: treat C5 + C6 as one mega-commit sequence, don't break them apart.

---

## w13-restructure conflict management

The audit was anchored at `5461ea7`, but workspace:13 is currently on phase 9i–9k of the crate restructure. By the time any phase of this plan fires, the file layout will have shifted. Key mitigations:

1. **Wait for workspace:13's "9k done" signal before starting any phase in this plan.** The manifest's execution status block in `docs/proposals/crate-restructure-manifest.md` will be updated when 9i–9k land; watch that.
2. **Re-grep every audit line:line reference before executing a phase.** The audit's file paths and line numbers were accurate at `5461ea7`; post-9k they may not be. Treat the audit as "here is the target surface," not "here are the exact sed coordinates."
3. **Do not execute C1 mechanically against the audit's file list until post-9k.** Some of the files C1 deletes (`crates/fireline-runtime/src/connections.rs`, the transports shims) are exactly the kind of compatibility veneer workspace:13 might already be deleting.
4. **All of C2–C6 touch `fireline-host/src/*` files that are moving.** Expect at least a rebase's worth of churn. Do not blame the plan if exact line numbers shift.

---

## Execution checklist (for the firing session)

- [ ] Confirm workspace:13's phases 9i, 9j, 9k are all landed on `origin/main`
- [ ] Confirm `cargo check --workspace` is green on `origin/main`
- [ ] Confirm the crate-restructure manifest's execution-status table has been updated to show phase 9 done
- [ ] **Get Q1 confirmed by the user before firing C5.** Everything else is incremental; C5 is a point of no return for the multi-child question.
- [ ] Fire C1 as a standalone commit first. Validate done-when checklist. Push.
- [ ] Fire C2. Validate. Push.
- [ ] Fire C3. Validate. Push.
- [ ] Fire C4 (or skip if merging into C5). Validate. Push.
- [ ] Fire CB any time after C2 (likely after C4).
- [ ] Fire C5 as a single PR with 4–6 commits. Validate each sub-phase's done-when before proceeding. Push.
- [ ] Fire C6. Validate. Push.
- [ ] Post-C6: the audit's target surface is closed. Run the audit's done-when greps one more time as an overall sanity check.

---

## Phases I judge need re-scoping

- **C5 is too large for a single commit.** Already decomposed into C5a–C5d above; treat each sub-phase as its own commit. **Do not squash.**
- **C4 may not justify a standalone phase.** If C5 is firing in the same session, fold C4's relocation into C5 as a delete-not-move. Decision criterion: is the interval between C4 and C5 more than one working day? If yes, run C4 as its own phase. If no, skip it.
- **CB may split into two.** The "rename Host entrypoints" part and the "delete the Category B dead verbs that go away with C5" part are orthogonal. CB as written assumes you run the renames once and then delete the rest in C5; if the user wants a single "naming cleanup" commit that includes the pre-deletion state, CB stays as one commit.

## Cross-phase dependencies the audit didn't flag

- **`bootstrap.rs` composes `auth.rs` middleware into the router assembly.** C5 deleting `auth.rs` without also patching the bootstrap composition will break the build mid-sequence. See "Cross-phase dependencies" above; C5 + C6 are tightly coupled at this boundary and should land as one PR sweep.
- **`fireline-runtime` compatibility shims are a cross-phase hazard.** `crates/fireline-runtime/src/connections.rs`, `runtime_host.rs`, `runtime_provider.rs`, `control_plane_peer_registry.rs`, and `bootstrap.rs` are all thin re-export veneers (audit:214). They serialize C1/C2/C4/C6 because each of those phases deletes a file that a shim re-exports. Either delete the shim first (a 5-line edit per file) or delete the target + the shim in the same commit. Do **not** leave a shim re-exporting a non-existent symbol between phases.
- **The `runtime_index_agreement` test is C5's verification anchor.** That test asserts the stream-derived projection observes the same runtime lifecycle that `RuntimeRegistry` observes today. Once C5c deletes the registry liveness methods, the test's comparison surface shifts; the test itself may need to be retargeted to compare "stream projection now" vs "stream projection after a cycle," not "stream projection" vs "registry." Confirm before committing C5d.
- **The CB rename of `fireline-sandbox::lib.rs::create` → `provision` crosses the `fireline-sandbox` crate boundary.** If that crate has external consumers beyond `fireline-host`, the rename is not a mechanical single-crate refactor. Confirm by grepping `fireline_sandbox::.*create` across the workspace before firing CB.
