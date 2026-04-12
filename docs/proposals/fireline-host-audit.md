# fireline-host architecture audit

> **SUPERSEDED** by [`./sandbox-provider-model.md`](./sandbox-provider-model.md) and [`./client-api-redesign.md`](./client-api-redesign.md). Retained for architectural history.
> **Status:** historical audit against the pre-provider-model host layout. Many findings here landed directly; the rest were overtaken by the provider-model rewrite.

## TL;DR

- Audit anchor: `origin/main` at `5461ea7` on 2026-04-11. The live worktree already has in-flight phase 9h edits in `crates/fireline-host/src/{bootstrap.rs,local_provider.rs,runtime_provider.rs}`, `src/main.rs`, and related tests; this report still flags them because most issues are outside that slice.
- `fireline-host` still carries a lot of pre-`6743bad` control-plane baggage: bearer-token issuance, register/heartbeat routes, stale-heartbeat scanning, and a file-backed `RuntimeRegistry` read path. Those all conflict with the per-Host + stream-as-truth target.
- The direct-host boot path is over-indirected: `src/main.rs` direct mode calls `RuntimeHost::create`, which constructs `BootstrapRuntimeLauncher`, which calls `bootstrap::start`, which then gets force-registered back through the same `register()` state machine. That split is not earning its keep.
- `bootstrap.rs`'s doc comment is stale. The code only merges ACP + embedded durable-streams routes; it does not merge runtime HTTP routes or file helper routes, and it still assumes embedded stream hosting that phase 9h is explicitly removing.
- `connections.rs` is dead, `routes_files.rs` is also still a TODO, and `build.rs` / `transports/websocket.rs` in `fireline-host` duplicate the live harness implementation.

## User's specific questions (with direct answers)

### 1. auth.rs — is this token machinery needed?

Short answer: **not in the target architecture as written; it is mostly leftover from the older "separate/global control plane" model and should be simplified away.**

What it protects today:

- `crates/fireline-host/src/auth.rs:27-75` issues and validates bearer tokens for `POST /v1/runtimes/{key}/register` and `POST /v1/runtimes/{key}/heartbeat`.
- `crates/fireline-host/src/router.rs:24-41,121-175` applies that middleware only to those two routes.
- The check is runtime-scoped, so one child runtime cannot register or heartbeat another runtime's key.

Why this looks stale:

- The launch path does **not** use the public token-issuance route. `ChildProcessRuntimeLauncher` injects a token directly from the in-process `RuntimeTokenStore` at launch time (`crates/fireline-host/src/local_provider.rs:136-144`).
- Docker does the same through `ControlPlaneTokenIssuer` (`crates/fireline-host/src/control_plane.rs:100-104,218-226`).
- The public `/v1/auth/runtime-token` route (`crates/fireline-host/src/router.rs:34,101-119`) is only exercised by tests and manual clients; it is not needed by the built-in launchers.
- The deployment doc's current direction is "one binary, per-Host HTTP API," not a federated control plane. In that model there is no meaningful multi-tenant boundary here; the host is authenticating its own children over localhost/per-Host infrastructure.

Recommendation:

- **Delete `auth.rs` as a standalone module and remove `/v1/auth/runtime-token`.**
- If `register` / `heartbeat` survive temporarily, fold a tiny launch-scoped shared-secret check directly into the control-plane code instead of keeping a reusable bearer-token subsystem.
- Longer-term, once stream-as-truth step 3 lands, delete `register` / `heartbeat` as externally meaningful callbacks too.

### 2. control_plane_peer_registry.rs — what does it do?

Short answer: **it is a thin HTTP-backed `PeerRegistry` adapter, not duplicated peer logic. Conceptually it belongs to Tools, not Host.**

What it does:

- `crates/fireline-host/src/control_plane_peer_registry.rs:23-33` fetches `GET /v1/runtimes`.
- `:37-54` implements `fireline_tools::directory::PeerRegistry`.
- `:57-82` filters runtime descriptors to ready-ish states and maps them into `fireline_tools::directory::Peer`.
- `crates/fireline-host/src/bootstrap.rs:150-155` injects it only when `control_plane_url` is present; otherwise bootstrap uses `LocalPeerDirectory`.

What it is not:

- It is **not** a second peer-registry abstraction. The trait already lives in `fireline-tools`; this file is just an adapter from runtime descriptors to that trait.
- It is **not** core Host logic. The consumer is `PeerComponent` in `fireline-tools` / `fireline-harness`, not the Host primitive itself.

Why it exists historically:

- When the control plane was a separate process, push-mode children needed some way to discover sibling runtimes without sharing the local peers file.
- `6743bad` mostly moved those files into `fireline-host`; it did not re-decide whether the adapter still belonged there.

Recommendation:

- **Move or rename it as a Tools-side satisfier** such as `HttpRuntimePeerRegistry`.
- If phase 9h / later cleanup removes child-to-control-plane peer discovery entirely, delete it with that path.
- Do not keep a file named `control_plane_peer_registry.rs` inside `fireline-host`; the name overstates what it is and the crate placement is wrong.

### 3. heartbeat.rs — still required?

Short answer: **no, not in the target model.**

What it does today:

- `crates/fireline-host/src/heartbeat.rs:5-26` is only a thin wrapper over `RuntimeRegistry`'s in-memory liveness map.
- `crates/fireline-host/src/control_plane.rs:107-187` runs a stale scanner that marks `Ready -> Stale` from that map.
- `crates/fireline-host/src/router.rs:140-175` records liveness on `register` / `heartbeat`.
- `crates/fireline-host/src/control_plane_client.rs:86-119` runs a best-effort heartbeat loop from the child runtime.

Why it is not needed in the target architecture:

- For the in-process direct-host path, self-heartbeating gives no signal that process supervision, ACP connectivity, or stream replay does not already give.
- For child runtimes, the host already owns the subprocess/container handle; that is a better liveness source than a second in-memory clock map.
- The stream-as-truth handoff already names this exact deletion: remove `RuntimeRegistry` + `HeartbeatTracker` + stale scanner once the read path flips.

Recommendation:

- **Delete `heartbeat.rs`.**
- Downstream updates: remove `HeartbeatTracker` from `control_plane.rs` and `router.rs`, remove `spawn_heartbeat_loop()` from `control_plane_client.rs`, remove the `/heartbeat` route, and drop the liveness methods from `RuntimeRegistry`.

### 4. runtime_provider vs local_provider — what's the split?

Short answer: **today the split is "in-process launcher" vs "child-process launcher," but only the child-process side looks architecturally justified. The direct-host side should collapse.**

What the two files are:

- `crates/fireline-host/src/runtime_provider.rs:11-61` defines `BootstrapRuntimeLauncher`, a `LocalRuntimeLauncher` impl that calls `bootstrap::start(...)` in-process.
- `crates/fireline-host/src/local_provider.rs:16-209` defines `ChildProcessRuntimeLauncher`, a `LocalRuntimeLauncher` impl that spawns the `fireline` binary, passes runtime env/config, and waits for readiness through `RuntimeRegistry`.

How the direct-host path currently works:

- `src/main.rs:138-166` direct mode calls `fireline_runtime::runtime_host::RuntimeHost::create(...)`.
- `crates/fireline-host/src/runtime_host.rs:20-25` constructs that wrapper with `BootstrapRuntimeLauncher`.
- `crates/fireline-host/src/runtime_provider.rs:15-53` turns around and calls `bootstrap::start(...)`.
- `crates/fireline-host/src/runtime_host.rs:35-55` then immediately calls `register()` if the inner host returns `Starting`.

That means direct-host mode is effectively:

`run_direct_host -> RuntimeHost::create -> BootstrapRuntimeLauncher -> bootstrap::start -> register()`

That is exactly the indirection the user called out. It is using the managed-runtime provider stack to start the one runtime the current process already is.

Is the abstraction earning its keep?

- **For child-process provisioning:** yes, somewhat. `LocalRuntimeLauncher` lets `LocalProvider` own resource preparation while swapping launch strategies.
- **For direct-host provisioning:** no. `BootstrapRuntimeLauncher` is a shim whose only job is to re-enter `bootstrap::start` through the same provider/registry state machine used for remote children.

Architectural placement:

- These launchers are **Host-internal runtime provisioning backends**.
- They are **not** the Sandbox primitive. `runtime-host-split.md` §7.3 is explicit on that point.
- The current dependency on `fireline_sandbox::{LocalRuntimeLauncher, RuntimeManager, RuntimeHost}` is crate-boundary drift, not a reason to keep the abstraction shape.

Recommendation:

- **Delete `BootstrapRuntimeLauncher` and the direct-host `runtime_host.rs` wrapper.**
- Call runtime bootstrap/runtime-server assembly directly from `src/main.rs` in direct mode.
- Keep the subprocess launcher only if Host still provisions child runtimes; rename it around process launch (`ChildProcessRuntimeLauncher` is clearer than `local_provider.rs`).

### 5. connections.rs — is this dead code?

Short answer: **yes.**

Evidence:

- `crates/fireline-host/src/connections.rs:1-37` is only a TODO stub.
- No production file imports it; the only hits are the module export in `crates/fireline-host/src/lib.rs:6` and the compatibility re-export in `crates/fireline-runtime/src/connections.rs:1-2`.
- The supposed replacement is not actually in place: `crates/fireline-resources/src/routes_files.rs:1-30` is also still a TODO stub, and `bootstrap.rs` does not merge any file-helper routes.

Recommendation:

- **Delete it outright.**
- Do not open a separate issue just to preserve this exact design; if workspace/file browsing is still required, re-spec it against `fireline-resources` / MCP / stream-backed resources instead of reviving the lookup-file idea.

### 6. bootstrap.rs — how does its role change?

Short answer: **today it is a stale mixed-purpose helper built around embedded durable-streams and local peer bootstrap. Under the new model it should shrink drastically or disappear.**

What it actually does today:

- `crates/fireline-host/src/bootstrap.rs:208-210` merges only:
  - `fireline_harness::routes_acp::router(app_state)`
  - `build_stream_router(...)`
- It does **not** merge runtime HTTP control-plane routes.
- It does **not** merge file helper routes.

So the top-of-file comment (`:3-15`) is already inaccurate.

What assumptions are stale under the new model:

- Embedded durable streams via `build_stream_router` (`:210`) instead of requiring an external durable-streams URL.
- Optional `external_state_stream_url` (`:57,129-133`) instead of a required `--durable-streams-url`.
- `StreamStorageConfig` / embedded storage selection (`:54,210`) even though the deployment doc moves durable streams out into its own artifact.
- Local peer-directory fallback (`:145-162,270-278`) as a bootstrap truth source.
- Emitting `runtime_spec_persisted` from inside bootstrap (`:227-262`) because direct-host still enters through the old runtime-provider state machine.

Should `bootstrap::start` exist at all?

- In the current codebase it mainly exists because both:
  - managed child runtimes call it directly (`src/main.rs:181-196`)
  - direct-host mode reaches it indirectly through `BootstrapRuntimeLauncher`
- Once direct-host stops tunneling through `RuntimeHost::create` and embedded durable streams are removed, that justification gets much weaker.

Recommendation:

- **Shrink `bootstrap::start` to the minimum runtime-server assembly needed for the managed child path, or inline it into `src/main.rs` once phase 9h settles.**
- The more the runtime startup becomes "build router from primitive crates, then `axum::serve` against an external durable-streams URL," the less value this helper provides.

## Category A — Cross-primitive leakage

| file:lines | belongs to | why it leaked into `fireline-host` | suggested target |
|---|---|---|---|
| `crates/fireline-host/src/control_plane_peer_registry.rs:1-82` | Tools | Push-mode children needed an HTTP-backed `PeerRegistry` when the control plane was separate; `6743bad` moved the adapter without re-homing it. | `fireline-tools` as an HTTP/runtime-list `PeerRegistry`, or delete with that path |
| `crates/fireline-host/src/build.rs:1-62` | Harness | Conductor-builder glue lived near runtime assembly before ACP routing moved into `fireline-harness`; the file survived as a copy. | Delete host copy; keep only the harness implementation |
| `crates/fireline-host/src/transports/websocket.rs:1-58` | Harness | ACP WebSocket transport logic is now owned by the live `/acp` route in `fireline-harness`, but the old host copy remains. | Delete host copy; keep only `fireline-harness` |
| `crates/fireline-host/src/transports/duplex.rs:1-26` | Harness test support | This is an ACP test transport, not Host assembly. It survives because `fireline-runtime` re-exports it for tests. | Move to `fireline-harness` or a test helper module |
| `crates/fireline-host/src/connections.rs:1-37` | Resources | It describes a workspace/files helper API contract, not Host lifecycle. It predates the current Resources + stream-backed direction and never landed. | Delete; if revived, re-spec in `fireline-resources` |
| `crates/fireline-host/src/bootstrap.rs:145-225` | Mixed Session / Harness / Tools composition | `6743bad` was mostly a mechanical merge, so runtime bootstrap still manually wires session projections, ACP state, and peer registry concerns inside a Host helper. | Shrink to thin runtime-server assembly; leave primitive logic in `fireline-session`, `fireline-harness`, and `fireline-tools` |

## Category B — Naming drift

| symbol | current | target primitive verb |
|---|---|---|
| `crates/fireline-host/src/router.rs:create_runtime` | `create` | Host `provision` |
| `crates/fireline-host/src/runtime_host.rs:create` | `create` | Host `provision` |
| `crates/fireline-sandbox/src/lib.rs:create` | `create` | Host `provision` |
| `crates/fireline-host/src/runtime_provider.rs:start_local_runtime` | `start` | Host `provision` at the primitive boundary; if kept internal, name it around process launch instead |
| `crates/fireline-host/src/local_provider.rs:start_local_runtime` | `start` | Host `provision` at the boundary; internal name should describe process launch, not leak a primitive surface |
| `crates/fireline-host/src/router.rs:register_runtime` | `register` | No Host primitive verb; make it internal stream/update plumbing or delete |
| `crates/fireline-host/src/router.rs:heartbeat_runtime` | `heartbeat` | No Host primitive verb; delete under stream-as-truth |
| `crates/fireline-host/src/control_plane_client.rs:spawn_heartbeat_loop` | `heartbeat` | No primitive verb; delete with the stale scanner path |

## Category C — Dead code

- `crates/fireline-host/src/connections.rs:1-37` is a dead exported TODO stub with no production callers.
- `crates/fireline-resources/src/routes_files.rs:1-30` is the matching dead TODO; the "connection lookup file" replacement is not implemented anywhere.
- `crates/fireline-host/src/build.rs:1-62` is dead in production. The live ACP route uses the harness copy in `crates/fireline-harness/src/routes_acp.rs:127-150`.
- `crates/fireline-host/src/transports/websocket.rs:1-58` and `crates/fireline-host/src/transports/duplex.rs:1-26` are dead in production; they survive only because tests import the `fireline-runtime` re-export veneer.
- `crates/fireline-host/src/router.rs:101-119` exposes `/v1/auth/runtime-token`, but built-in launchers never use it.
- `crates/fireline-host/src/router.rs:254-260` accepts `_scope` on token issuance and then ignores it.
- `crates/fireline-runtime/src/connections.rs:1-2` is a compatibility shim over the dead host stub, complete with `#[allow(unused_imports)]`.
- `crates/fireline-sandbox/src/provider.rs:11-19` stores `RuntimeLaunch.status`, but nothing ever reads that field.

## Category D — Duplication

- `crates/fireline-host/src/build.rs:39-62` duplicates `crates/fireline-harness/src/routes_acp.rs:127-150`.
- `crates/fireline-host/src/transports/websocket.rs:18-58` duplicates `crates/fireline-harness/src/routes_acp.rs:152-188`.
- `crates/fireline-host/src/control_plane.rs:190-200` and `crates/fireline-host/src/bootstrap.rs:297-305` both normalize bind addresses into connect addresses.
- `now_ms()` is duplicated in `crates/fireline-host/src/auth.rs:91-96`, `crates/fireline-host/src/router.rs:274-279`, `crates/fireline-host/src/control_plane.rs:211-216`, `crates/fireline-host/src/control_plane_client.rs:122-127`, and `src/main.rs:309-314`.
- `crates/fireline-runtime/src/{bootstrap.rs,runtime_host.rs,runtime_provider.rs,control_plane_peer_registry.rs,connections.rs}` are mostly thin re-export veneers over `fireline-host`, so the crate boundary is duplicated even where behavior is not.
- `crates/fireline-host/src/runtime_host.rs:35-55` duplicates the managed-runtime registration path by immediately calling `register()` after an in-process launch.

## Category E — Stream-as-truth violations

- `crates/fireline-host/src/control_plane.rs:67-118` and `crates/fireline-host/src/router.rs:49-99,121-175` still serve and mutate runtime state through file-backed `RuntimeRegistry`, not the stream-derived `RuntimeIndex`.
- `crates/fireline-host/src/local_provider.rs:61-100` determines readiness by polling `RuntimeRegistry`, so the control plane can disagree with the runtime's durable stream.
- `crates/fireline-host/src/heartbeat.rs:5-26` and `crates/fireline-host/src/control_plane.rs:107-187` drive `Ready <-> Stale` transitions from an in-memory liveness map instead of stream projection.
- `crates/fireline-sandbox/src/registry.rs:13-15,71-98` keeps liveness in `Arc<Mutex<HashMap<String, i64>>>`; that state is invisible to the stream and disappears on restart.
- `crates/fireline-sandbox/src/lib.rs:45-50,117-158,210-247,290-305` keeps `live_handles` and `pending_runtime_specs` in memory. That is exactly the divergence smell already called out in `runtime-host-split.md`.
- `crates/fireline-host/src/runtime_host.rs:35-55` marks direct-host runtimes `Ready` by reusing the register path instead of deriving readiness from the runtime's own observable lifecycle.

## Recommended follow-up lanes

1. Delete the dead surfaces: `connections.rs`, `routes_files.rs`, the `fireline-runtime` connection shim, and the duplicate host `build` / transport copies.
2. Collapse the direct-host path so it no longer goes through `RuntimeHost::create -> BootstrapRuntimeLauncher -> register()`.
3. Finish the stream-as-truth read-path flip, then delete `HeartbeatTracker`, the stale scanner, and registry-backed heartbeat transitions.
4. Remove `/v1/auth/runtime-token` and the reusable bearer-token subsystem; if register/heartbeat survive briefly, validate a launch-scoped secret inline.
5. Move or delete `ControlPlanePeerRegistry`; if it survives, rename it as a Tools-side HTTP `PeerRegistry` adapter.
6. Rewrite `bootstrap.rs` around a required external durable-streams URL and decide whether any helper beyond `src/main.rs` assembly is still justified.

## Open questions

- Does phase 9h still intend to support multi-runtime child provisioning on a single Host, or is the target simplifying toward a direct-host-only posture plus external durable streams? That answer decides whether `register` / `heartbeat` are transitional or fundamentally dead.
- Is peer discovery supposed to remain "ask the local host over HTTP for sibling runtimes," or should it move to a stream-backed / tool-native discovery surface?
- Is the browser/workspace file-helper REST API still a real requirement, and if so should it be revived as REST at all instead of as Resources/MCP functionality?
