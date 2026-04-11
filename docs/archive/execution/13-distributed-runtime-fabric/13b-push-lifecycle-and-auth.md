# 13b: Push Lifecycle and Auth

Status: planned
Type: execution slice

Related:

- [`./README.md`](./README.md)
- [`./phase-0-runtime-host-and-peer-registry-refactor.md`](./phase-0-runtime-host-and-peer-registry-refactor.md)
- [`./13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md`](./13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md)
- [`./13c-first-remote-provider-and-mixed-topology.md`](./13c-first-remote-provider-and-mixed-topology.md)
- [`../../runtime/control-and-data-plane.md`](../../runtime/control-and-data-plane.md)
- [`../../runtime/heartbeat-and-registration.md`](../../runtime/heartbeat-and-registration.md)

## Objective

Replace the polling readiness model with a push-based lifecycle protocol —
register, heartbeat, state machine — and put bearer auth on the new control
plane write surface.

This slice ships against `LocalProvider` only. No remote providers. Polling
remains the default; push is opt-in via a launcher flag, so the existing
browser-harness e2e keeps passing on the old path.

This is the foundation that the first remote provider (`13c`) requires. It is
deliberately scoped down from what the old single-doc `13b` tried to bundle so
that push protocol, state machine, and auth can be reviewed as one coherent
unit before any non-local launcher lands.

## Product Pillar

Provider-neutral runtime fabric.

## User Workflow Unlocked

No new product surface. This is contract work that unlocks every later
provider.

The user-visible effect is invariant preservation: a runtime in `ready` is a
real promise that ACP and state-stream connections will succeed, and a runtime
in `stale` or `broken` is a real signal that they will not.

## Why This Comes Before Any Remote Provider

The phase 1 control plane uses a polling readiness model: the launcher spawns
a runtime subprocess, then polls a shared `runtimes.toml` file every 100ms
until the runtime writes `RuntimeStatus::Ready`. That works because the
control plane and the runtime share a filesystem.

It silently breaks the moment a runtime runs in a place that doesn't share
the control plane's filesystem — a Docker container, an E2B sandbox, a
Cloudflare worker, a remote pod. The shared file is no longer reachable, the
poll loop never sees `Ready`, and startup times out.

The fix is documented end-to-end in
[`heartbeat-and-registration.md`](../../runtime/heartbeat-and-registration.md):
the runtime calls `POST /v1/runtimes/{key}/register` over HTTP after its
listeners are bound, and the control plane uses the registration call as the
trigger for the `starting → ready` transition. From then on, the runtime
heartbeats every 5s; the control plane marks it `stale` after 30s without a
heartbeat and `broken` if the launcher reports a child exit.

The wire protocol is the same regardless of where the runtime runs. That means
this slice — push protocol plus auth — is a precondition for every non-local
provider, not a Docker-specific thing.

## Scope

### 1. Gate the readiness transition in the runtime host

The single highest-leverage code change is in
`crates/fireline-conductor/src/runtime/mod.rs`. Today `RuntimeHost::create()`
unconditionally transitions `starting → ready` immediately after
`RuntimeProvider::start()` returns, on the assumption that the launcher
already proved readiness via polling.

This slice changes that: provider start returns a descriptor whose status
remains `starting`. The transition to `ready` happens only when a successful
`/register` call arrives for the corresponding `runtime_key`. The polling
launcher path keeps working by emitting an internal "register" once it
confirms readiness via the file, so the e2e test stays green.

This is the seam the entire push lifecycle hangs on. Implement it first.

### 2. Push endpoints on the control plane

Add to `crates/fireline-control-plane`:

```text
POST /v1/runtimes/{runtimeKey}/register    body: RuntimeRegistration  → 200 / 401 / 409
POST /v1/runtimes/{runtimeKey}/heartbeat   body: HeartbeatReport      → 200 / 401 / 410
```

Per `heartbeat-and-registration.md` §"Endpoints (on the control plane)" and
§"Status state machine":

- `RuntimeRegistration` carries `runtime_id`, `node_id`, `provider`,
  `provider_instance_id`, `advertised_acp_url`, `advertised_state_stream_url`,
  `helper_api_base_url`, and `capabilities`.
- `/register` is upsert-shaped on `runtime_key`. Existing `starting` records
  transition to `ready`; existing `ready` records refresh fields (this is
  what makes control-plane restart transparent); `stopped` records return 409.
- `/heartbeat` is fire-and-forget. The control plane records
  `last_heartbeat_at` per runtime and keeps an in-memory `HeartbeatTracker`.
- A scheduled task scans the tracker every few seconds and transitions
  runtimes whose last heartbeat is older than the stale threshold from
  `ready` to `stale`. A subsequent heartbeat moves them back to `ready`.

State machine and transitions are exactly as documented in
`heartbeat-and-registration.md` §"Status state machine" — do not invent a new
shape.

### 3. Bearer auth on the push surface

Per `control-and-data-plane.md` §2 and `heartbeat-and-registration.md`
§"Authentication":

- Add `POST /v1/auth/runtime-token` to the control plane returning
  `{ token, expires_at }` scoped to a `runtime_key`.
- Add bearer middleware that validates `Authorization: Bearer <token>` on
  `/register` and `/heartbeat`.
- Server-side enforcement: a token issued for runtime A must not be accepted
  on `/register` or `/heartbeat` for runtime B. This is enforced on the
  control plane, not in the runtime.
- The launcher passes the token to spawned runtimes via the
  `FIRELINE_CONTROL_PLANE_TOKEN` environment variable.

Token format for this slice: shared-secret opaque bearer. Rotation, JWT,
mTLS, and federation are deferred — see `control-and-data-plane.md` §9.

### 4. Runtime-side push client

Add `src/control_plane_client.rs` in the runtime binary, following the sketch
in `heartbeat-and-registration.md` §"Runtime-side client sketch":

- `ControlPlaneClient::register(RuntimeRegistration)` with retry/backoff
  (250ms → 2s, 3 attempts) and 2-second per-attempt timeout
- `spawn_heartbeat_loop(...)` that posts every 5 seconds with best-effort
  semantics (single failures logged, do not abort the loop)
- Picks up `FIRELINE_CONTROL_PLANE_URL` and `FIRELINE_CONTROL_PLANE_TOKEN`
  from the environment

Wire it into `run_managed_runtime()` in `src/main.rs`. Ordering at startup is
**not negotiable** and follows `heartbeat-and-registration.md`
§"Ordering at runtime startup":

1. Parse args, load config
2. Bring up the durable-streams producer
3. Bind the axum listener
4. Verify the listener is accepting connections
5. Start the runtime-local materializer subscription and preload
6. **Call `ControlPlaneClient::register()`** — only after steps 3–5 succeed
7. Start the heartbeat loop
8. Begin serving traffic

### 5. LocalProvider opt-in flag

Add `LocalProvider::prefer_push: bool`, defaulting to `false`. When `true`,
the launcher spawns runtimes with `--control-plane-url` set and stops polling
the registry file; the runtime's own `/register` call drives the transition
to `ready`.

The default stays `false` so the existing polling path and the existing
browser-harness e2e do not regress.

### 6. Consolidate the dual launcher surfaces

There are currently two launchers in the tree:

- `crates/fireline-control-plane/src/local_provider.rs`
- `src/runtime_provider.rs`

The push work must land in the shared
`crates/fireline-conductor/src/runtime/` path so both launchers consume it.
Without consolidation, push would only work in one of them and the contract
would silently bifurcate.

### 7. Update §4a with rule 6

Edit `docs/runtime/control-and-data-plane.md` §4a to add the new readiness
invariant per `heartbeat-and-registration.md` §"Invariant preservation vs
§4a":

> 6. **`stale` and `broken` are not-ready states.** A runtime that has missed
>    its heartbeat threshold or whose provider reports failure does not
>    satisfy rule 2's data-plane promise. Consumers must treat these the same
>    as `starting` for the purpose of deciding whether to open `/acp` or
>    state-stream subscriptions.

## Explicit Non-Goals

This slice does **not** add:

- `DockerProvider`, E2B, Daytona, Cloudflare, or any non-local provider
- Mixed topology proof (1 local + N remote runtimes)
- Multi-stream observation in TS clients
- Cross-runtime peer call proof
- A control-plane lifecycle event stream or `RuntimeRegistryProjector`
  (deferred per `control-and-data-plane.md` §2 storage notes)
- Token rotation, JWT, or mTLS
- Auto-restart or remediation policy on `stale` / `broken`
- Removing the polling code path

Each of those is correctly placed in `13c` or later. Bundling them here is
what made the old single-doc 13b too large to hand off cleanly.

## Files Likely Touched

Rust:

- `crates/fireline-conductor/src/runtime/mod.rs` — gate the readiness flip,
  add `register` and `heartbeat` methods
- `crates/fireline-conductor/src/runtime/local.rs` — `prefer_push: bool`
- `crates/fireline-conductor/src/runtime/provider.rs` — pass through any new
  fields the registration body needs
- `crates/fireline-control-plane/src/main.rs` — new routes
- `crates/fireline-control-plane/src/heartbeat.rs` — new module
- `crates/fireline-control-plane/src/auth.rs` — new module
- `crates/fireline-control-plane/src/local_provider.rs` — wire `prefer_push`
- `src/main.rs` — wire push client into `run_managed_runtime`
- `src/control_plane_client.rs` — new file
- `src/runtime_provider.rs` — consolidate against shared conductor path

Docs:

- `docs/runtime/control-and-data-plane.md` — §4a rule 6

## Acceptance Criteria

- `RuntimeHost::create()` no longer auto-flips `starting → ready` after
  `provider.start()` returns; the transition is gated on a successful
  `/register` call (or the polling launcher's internal register-equivalent
  for backwards compat)
- `POST /v1/runtimes/{key}/register` accepts a `RuntimeRegistration` body
  carrying `provider`, `provider_instance_id`, `advertised_acp_url`,
  `advertised_state_stream_url`, and `helper_api_base_url`, and the resulting
  `RuntimeDescriptor` reflects those fields (not whatever the launcher
  passed in)
- `POST /v1/runtimes/{key}/heartbeat` updates `last_heartbeat_at` and is
  fire-and-forget
- A `HeartbeatTracker` background task transitions a `ready` runtime to
  `stale` after 30s without a heartbeat, and back to `ready` on the next
  successful heartbeat
- `/register` and `/heartbeat` reject requests without a valid bearer token
  (test asserts 401)
- A token issued for runtime A is rejected on `/register` and `/heartbeat`
  for runtime B (test asserts 401 or 403)
- `POST /v1/auth/runtime-token` issues a token scoped to a `runtime_key`
- `LocalProvider::prefer_push: bool` defaults to `false`; when `true`, the
  launcher spawns runtimes with `--control-plane-url` set and the runtime
  registers via HTTP instead of writing to `runtimes.toml`
- The browser-harness e2e passes against both `prefer_push: false` (polling)
  and `prefer_push: true` (push), parameterized by an env var
- §4a in `control-and-data-plane.md` includes rule 6 for `stale` and
  `broken` as not-ready states

## Validation

- `cargo test -q`
- `pnpm --filter @fireline/client test`
- one Rust integration test that:
  - starts the control plane
  - issues a runtime token via `POST /v1/auth/runtime-token`
  - asserts `/register` returns 401 without the token
  - asserts `/register` returns 401 when the token is for a different
    `runtime_key`
  - registers successfully with the right token, asserts the descriptor
    transitions to `ready`
  - sends a heartbeat, asserts `last_heartbeat_at` advances
  - withholds heartbeats for >30s, asserts the descriptor transitions to
    `stale`
  - resumes heartbeats, asserts the descriptor transitions back to `ready`
- one runtime test that exercises the consolidated launcher path against
  both `prefer_push: false` and `prefer_push: true`
- the existing browser-harness e2e parameterized to run against both modes

## Handoff Note

This slice is the precondition for `13c`. Without it, every non-local
provider has to invent its own readiness signaling and the runtime contract
silently bifurcates per provider.

Implementation order matters: do step 1 (gate the readiness transition)
first. The rest of the slice depends on that single change being in place.

Coordinate the auth surface (`/v1/auth/runtime-token` + bearer middleware)
with the agent building the `/register` and `/heartbeat` routes — they should
land in the same router module to avoid ordering races.
