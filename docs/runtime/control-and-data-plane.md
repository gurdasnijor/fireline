# Control Plane and Data Plane Split

> Status: architectural decision
> Type: reference doc (not an execution slice)
> Audience: maintainers deciding where new runtime/fabric code should live
> Related:
> - [`../execution/12-programmable-topology-first-mover.md`](../execution/12-programmable-topology-first-mover.md)
> - [`../execution/next-steps-proposal.md`](../execution/next-steps-proposal.md)
> - [`../mesh/peering-and-lineage.md`](../mesh/peering-and-lineage.md)
> - [`../state/runtime-materializer.md`](../state/runtime-materializer.md)

## Purpose

Fireline has been thrashing on "where does the control plane live vs the data plane" for several slices running. This doc commits to one answer per question so future slices can reference these definitions instead of reopening them.

Each section below is a decision, not an option list. Push back on any specific decision you disagree with — but the intent is that after this, every future slice (12, 13, and beyond) can treat these definitions as settled.

## 1. The two planes, defined by what they carry

**Control plane** carries **metadata and lifecycle**: *what runtimes exist, who can talk to whom, what tokens are valid, what topology a runtime should run, what providers are available*. Traffic is infrequent (runtime creation, registration, heartbeats, descriptor lookups). It never touches session payloads. **One process per environment.**

**Data plane** carries **actual agent work**: ACP requests and responses, session notifications, MCP tool calls, trace events, durable state rows, helper-API file reads. Traffic is hot, per-session, possibly high-volume. **Many processes per environment** — one per runtime, plus one shared durable-streams deployment.

### Litmus test

For any new surface, ask:

1. *Does this describe what should happen, or does it carry what is happening?*
   → control plane if the former, data plane if the latter.
2. *Does this live once per environment, or once per runtime (or per stream)?*
   → control plane if once per environment, data plane if per runtime.
3. *Is this load-bearing for session throughput?*
   → data plane if yes.

## 2. Control plane — concrete spec

### Binary

**`fireline-control-plane`** — a new binary, lives at `crates/fireline-control-plane/` with `src/main.rs` and an axum Router. Reuses existing types from `fireline-conductor` and the runtime host / provider types extracted out of `src/runtime_host.rs` (see [phase 0](#8-phased-delivery) below). The control plane is a thin HTTP wrapper around the existing `RuntimeHost` shape plus an in-memory registry — **not a new framework and not a new abstraction layer**.

### HTTP API surface

All JSON bodies. Auth via `Authorization: Bearer <control_plane_token>` on every request.

**Runtime lifecycle:**

```text
POST   /v1/runtimes                        body: CreateRuntimeSpec    → RuntimeDescriptor
GET    /v1/runtimes                        query: ?provider=&status=  → RuntimeDescriptor[]
GET    /v1/runtimes/{runtimeKey}                                       → RuntimeDescriptor
POST   /v1/runtimes/{runtimeKey}/stop                                  → RuntimeDescriptor
DELETE /v1/runtimes/{runtimeKey}                                       → RuntimeDescriptor (last known)
```

`CreateRuntimeSpec` is the same struct as today's local `CreateRuntimeSpec` in `src/runtime_host.rs`, plus:

- `topology: Option<TopologySpec>` (slice 12's output)
- `provider: ProviderRequest` (`local`, `docker`, future `cloudflare`, etc.)

The response `RuntimeDescriptor` carries endpoint objects, not URL strings:

```ts
type Endpoint = {
  url: string
  headers?: Record<string, string>
}

type RuntimeDescriptor = {
  runtimeKey: string
  runtimeId: string
  nodeId: string
  provider: "local" | "docker" | "cloudflare"
  providerInstanceId: string
  status: "starting" | "ready" | "busy" | "idle" | "stale" | "broken" | "stopped"
  acp: Endpoint        // already carries bearer token in headers
  state: Endpoint      // already carries bearer token in headers
  helperApiBaseUrl?: string
  createdAtMs: number
  updatedAtMs: number
}
```

`RuntimeDescriptor.status` has strict startup semantics:

- `starting` means the control plane has accepted the lifecycle request, but callers must assume the runtime's ACP endpoint and state endpoint are not yet reachable.
- `ready` means the runtime has registered its final advertised endpoints and those endpoints are now valid data-plane entrypoints for ACP and state observation.
- `stopped`, `stale`, and `broken` mean callers must not attempt new data-plane work against that descriptor unless a later control-plane read returns `ready` again.

**Runtime self-registration and heartbeat** (called by runtimes, not by TS clients):

```text
POST /v1/runtimes/{runtimeKey}/register    body: RuntimeRegistration  → empty
POST /v1/runtimes/{runtimeKey}/heartbeat   body: HeartbeatReport      → empty
```

`RuntimeRegistration` is what the runtime publishes at boot: its real `runtime_id`, provider kind, provider instance id, final `advertised_acp_url`, final `state_stream_url`, readiness flag. The control plane writes this into the record and transitions `starting → ready`.

`HeartbeatReport` is minimal — current timestamp plus optional load metrics. Cadence and timeouts per slice 13:

- heartbeat period: **5 seconds**
- stale timeout: **30 seconds** without heartbeat
- `stale` and `broken` records remain queryable until explicit `stop` / `delete`
- automatic record deletion is out of scope for the first cut

**Auth (token issuance):**

```text
POST /v1/auth/runtime-token    body: { runtime_key, scope }   → { token, expires_at }
```

The control plane is the token authority. ACP endpoints and durable-stream endpoints both validate bearer tokens issued here. The control plane never validates session content — it only issues and revokes.

**Component catalog** (future, not part of slice 13 v1):

```text
GET /v1/components                        → [{ name, kind, schema_url }]
GET /v1/components/{name}/schema          → JSON Schema
```

### What the control plane owns

- The authoritative catalog of runtimes (in-memory `HashMap` for slice 13 v1, persistent store later).
- `RuntimeProvider` implementations (`LocalProvider`, `DockerProvider`, future `CloudflareProvider` and `KubernetesProvider`).
- Bearer token minting and validation.
- Heartbeat tracking and `stale` / `broken` transitions.
- The "who are the peers" answer — the runtime catalog *is* the peer directory for multi-host topologies.

### What the control plane does **not** own

- Any ACP traffic (not even passing through it).
- Any trace events or durable state rows.
- Any MCP tool calls.
- Agent subprocess lifecycle — the control plane tells providers to spawn runtimes; the runtime itself owns its agent child.
- Session content.
- `LoadCoordinatorComponent` and `session/load` coordination — those are runtime-local concerns.

### Storage

- **Slice 13 v1:** in-memory `HashMap<runtime_key, RuntimeRecord>` guarded by `Arc<Mutex<_>>`. Same shape as today's `RuntimeHost::live_handles`. Control plane restart means the registry is rebuilt from runtime self-re-registration.
- **Later:** SQLite or Postgres for persistence. Or dogfood: the control plane's registry can itself be projected from a durable-streams stream (events like `RuntimeRegistered`, `RuntimeStopped`, `HeartbeatSeen`) with an in-memory projection. Forward-looking, not slice 13 scope.

## 3. Data plane — concrete spec

The data plane is **three kinds of process**, not one. They are all data plane because they all carry session-adjacent traffic — but they play three distinct *roles* that are worth naming explicitly before the per-process specs.

### Three roles

```text
                ┌─────────────────────────────────┐
                │  COMPUTE — runtime processes    │
                │                                 │
                │  fireline binary × N            │
                │   - accepts ACP connections     │
                │   - runs agent subprocess       │
                │   - EMITS trace events          │
                └────────────┬────────────────────┘
                             │ writes
                             ▼
                ┌─────────────────────────────────┐
                │  PERSISTENCE — durable streams  │
                │                                 │
                │  durable-streams-server × 1     │
                │   - STORES events durably       │
                │   - BROADCASTS via SSE          │
                │   - one stream per runtime      │
                └────────────┬────────────────────┘
                             │ reads
                             ▼
                ┌─────────────────────────────────┐
                │  CONSUMERS — materializers      │
                │                                 │
                │  runtime-local (inside compute):│
                │   - SessionIndex                │
                │   - ActiveTurnIndex             │
                │   - RuntimeMaterializer         │
                │                                 │
                │  external:                      │
                │   - TS StreamDB (browser)       │
                │   - Flamecast observer          │
                │   - audit sinks                 │
                └─────────────────────────────────┘
```

- **Compute** — the fireline runtime binaries. Ephemeral, many, spawned and torn down by the control plane. Accept ACP connections, host agent subprocesses, emit trace events. See §3a.
- **Persistence** — the shared durable-streams deployment. Long-lived, one-per-environment, stateful, operationally independent of the control plane and of any individual runtime. Stores events durably, broadcasts via SSE. See §3b.
- **Consumers** — materializers that *read* events and project them into queryable views. Some live inside compute processes (the runtime-local `SessionIndex`, `ActiveTurnIndex`, `RuntimeMaterializer`); others are external (TS `StreamDB` in the browser, Flamecast, audit sinks, future observers). Consumers are cross-cutting and don't have a dedicated subsection — they live wherever a query needs to be served.

**Materialization happens in consumers, not in the persistence tier.** The DS server doesn't know what an event means — it stores bytes and broadcasts them. Every projection, every derived state, every "what's the active turn for session X?" answer is computed by a consumer reading from its subscription. This is what lets per-runtime streams and runtime-local materializers compose cleanly: the persistence layer has no opinion about entity types, so consumers can layer whatever projections they need without server-side schema coordination.

Peer-to-peer ACP traffic (§3c) is a fourth wire shape — compute-to-compute traffic that bypasses persistence entirely. It's still data plane (it carries session payloads), but it doesn't participate in the three-role flow above.

### 3a. Fireline runtime binary (existing `fireline`)

**One process per runtime.** Spawned by the control plane via a provider (`LocalProvider` = subprocess, `DockerProvider` = container). Receives its configuration via environment variables at boot:

```text
FIRELINE_CONTROL_PLANE_URL          where to register and heartbeat
FIRELINE_CONTROL_PLANE_TOKEN        bootstrap credential for registration
FIRELINE_RUNTIME_KEY                pre-assigned stable ID
FIRELINE_NODE_ID                    operator-assigned node identity
FIRELINE_ADVERTISED_ACP_URL         what to register as our ACP endpoint
FIRELINE_EXTERNAL_STATE_STREAM_URL  external DS endpoint (absent = embedded mode)
FIRELINE_AGENT_COMMAND              agent subprocess command line
FIRELINE_TOPOLOGY                   serialized TopologySpec (slice 12)
```

**HTTP surface of the runtime:**

```text
GET  /healthz                       readiness probe
GET  /acp                           WebSocket — the real ACP endpoint
GET  /api/v1/files/{...}            helper file API
/v1/stream/{name}                   ONLY in embedded-DS mode; absent otherwise
```

**What the runtime owns:**

- The agent subprocess, via `SharedTerminal`.
- The ACP conductor, proxy chain, and any registered topology components.
- Per-session in-memory state.
- The `trace.rs` producer, writing either to a local embedded DS (dev mode) or to an external shared DS (control-plane mode).
- Runtime-local materializers reading only its own stream.
- MCP bridges for peer calls and any host-tool MCP servers.

**What the runtime does not own:**

- Runtime lifecycle decisions — the control plane decides when it should exist and when it should die (via `SIGTERM`).
- Its own stable identity — the control plane assigns `runtime_key`.
- The authoritative catalog of itself or its peers — it registers with the control plane and reads peers from the control plane.
- Any other runtime's stream.

### 3b. Shared durable-streams deployment

**The persistence tier.** Functionally, a **durable append-only log with publish/subscribe** — think "Kafka's storage tier with SSE as the subscription transport, minus the partitioning." One process or cluster per environment. Runs the upstream `durable-streams-server` binary unchanged; Fireline does not fork or extend it.

#### What it IS

- An append-only log server that stores event bytes durably
- A fan-out point for SSE (or long-poll) subscriptions
- The single source of truth for "what happened" in an environment
- The longest-lived infrastructure in the stack — survives every runtime restart, every control plane restart, every consumer reconnect

#### What it is NOT

- **Not a stream processor.** It does not transform, filter, route between streams, or join. Bytes go in; the same bytes come out to subscribers in the order they were written.
- **Not a materialization layer.** Materialization happens in consumers (runtime-local projections, TS `StreamDB`, Flamecast, audit pipelines). The DS server has no schema awareness and no opinion about what an event means.
- **Not Fireline-aware.** The DS server knows nothing about Fireline's concepts — `TraceEnvelopeV2`, `SessionIndex`, ACP lineage, topology, peers. Every piece of Fireline semantics lives in Fireline's code on both ends of the wire (producer at write time, consumer at read time).
- **Not a control-plane component.** The DS deployment does not know about the control plane. The control plane does not manage the DS deployment's lifecycle. They are independent services that happen to trust tokens issued by the same authority.

#### Stream naming

One stream per runtime, named by `runtime_key`:

```text
https://ds.example.com/v1/stream/fireline-state-runtime-a
https://ds.example.com/v1/stream/fireline-state-runtime-b
https://ds.example.com/v1/stream/fireline-state-runtime-c
```

This is a deliberate choice over "one global stream for all runtimes." Per-runtime streams mean each runtime-local materializer's subscription has simple replay boundaries, each consumer knows exactly what scope it's observing, and the persistence tier doesn't have to carry any Fireline-specific filtering logic. A global aggregate stream may be valuable later for cross-runtime observers; it should be additive, not a replacement.

#### The two wire paths

The DS server participates in the data-plane API conversation via exactly **two HTTP paths**:

1. **Writes (from runtimes).** `POST` / `PUT` against `/v1/stream/{name}` with JSON bodies. One write per trace event, one stream per runtime. Produced by `fireline_conductor::trace::DurableStreamTracer` via a `durable_streams::Producer` handle pointed at the stream URL. `FIRELINE_EXTERNAL_STATE_STREAM_URL` on the runtime side picks whether the producer targets an embedded DS (dev mode) or the shared deployment (control-plane mode).

2. **Reads (from consumers).** `GET` with SSE or long-poll against `/v1/stream/{name}`, with `offset` and `live` parameters. One SSE subscription per consumer per stream. Consumed by:
   - Runtime-local `RuntimeMaterializer` reading its own runtime's stream to populate `SessionIndex`, `ActiveTurnIndex`, and any future projection.
   - TS `@durable-streams/state` `StreamDB` for browser-side reactive queries.
   - Future: Flamecast cross-runtime observer reading N runtime streams in parallel and stitching them by lineage fields (`traceId`, `parentPromptTurnId`).

Neither path goes through the control plane. Neither path requires the control plane to be reachable once the DS endpoint is known. **The control plane is the discovery step; the DS server is the data step.** TS clients call `client.host.get(runtimeKey)` to receive a `RuntimeDescriptor` whose `state: Endpoint` carries the DS URL + auth header; from that moment on, all reads flow directly between consumer and DS server without the control plane in the loop.

#### Auth

Bearer tokens issued by the control plane. The DS server's existing auth hooks validate them; there is no Fireline-specific proxy in front of it. The control plane mints tokens scoped to specific streams (write-to-own-stream for runtimes, read-from-N-streams for observers); the DS server validates against whatever scheme the control plane signs. See §2's token issuance endpoint.

#### Independence and lifetime

The DS deployment has its own lifecycle, independent of the control plane and of any runtime:

- It starts before the control plane does, or alongside; ordering isn't strict because the control plane never calls the DS server.
- It survives control plane restarts unchanged — runtimes keep writing, consumers keep reading.
- It survives runtime restarts unchanged — a new runtime process with the same `runtime_key` continues writing to the same stream; the stream persists across the gap.
- Scaling, replicating, or replacing the DS deployment (say, moving from in-memory dev mode to a clustered production deployment) should not require touching any other tier. Consumers reconnect; writers reconnect; nothing else cares.

**This independence is the load-bearing reason** we chose "one deployment per environment, one stream per runtime" over "one embedded DS per runtime." The embedded model ties persistence to compute lifetime, which breaks the moment a runtime restarts and the stream it wrote to vanishes with it. The shared-deployment model makes persistence durable across every other tier in the stack, and it's what lets consumers like Flamecast observe a mixed multi-runtime fabric without needing to coordinate with compute-tier lifecycle at all.

### 3c. Peer-to-peer ACP traffic

When runtime A's `PeerComponent` handles a `prompt_peer` tool call, it opens a **fresh ACP WebSocket client** directly to runtime B's `/acp` endpoint. This traffic:

- Travels **directly between runtimes**, not through the control plane.
- Uses the target runtime's advertised endpoint from the control plane's catalog — read once at tool-call time.
- Is authenticated by a bearer token that runtime A obtained from the control plane.
- Carries lineage via `_meta.fireline.*` on the `initialize` handshake — the existing Spike 5 design.

**This is data plane. The control plane never sees `session/prompt` traffic or response payloads.**

## 4. The protocol between the planes

```text
startup:    runtime ── POST /v1/runtimes/{key}/register  ──→ control plane
steady:     runtime ── POST /v1/runtimes/{key}/heartbeat ──→ control plane   (5s)
shutdown:   control plane ── SIGTERM ──────────────────────→ runtime
discover:   TS client ──── GET  /v1/runtimes ──────────────→ control plane   → RuntimeDescriptor[]
acp:        TS client ──── WebSocket ws://.../acp ─────────→ runtime         (data plane)
state:      TS client ──── SSE https://.../v1/stream/... ──→ durable-streams (data plane)
peer-call:  runtime A ──── WebSocket ws://B/acp ───────────→ runtime B       (data plane)
token:      anyone    ──── POST /v1/auth/runtime-token ────→ control plane   → bearer token
```

Every data-plane link carries a bearer token in `Authorization:`. Every control-plane link does too. The control plane is the only thing that can mint tokens. Data plane processes only validate them.

## 4a. Startup and readiness invariants

These rules are the part Fireline has been leaving implicit. They are now explicit.

1. **The control plane owns runtime existence.**
   No client should infer runtime existence from a port, a process, or a dev proxy. The source of truth is a `RuntimeDescriptor` read from the control plane or a control-plane-backed adapter.

2. **`ready` is a data-plane promise, not a process-state hint.**
   `ready` means "it is now valid to open ACP and state-plane connections against the advertised endpoints." It does not mean merely "spawn requested" or "child process started."

3. **Frontends must not probe the data plane speculatively.**
   A UI must not eagerly open `/acp`, subscribe to `/v1/stream/...`, or construct long-lived state observers until it has a descriptor whose status is `ready`.

4. **Dev-mode proxies do not change the contract.**
   A same-origin Vite proxy from `/acp` or `/v1/stream/*` to a local runtime is only a transport convenience. It is not a readiness signal. The browser harness still has to wait for the control surface to report `ready` before touching those proxied routes.

5. **The control plane returns discovery material, the data plane carries work.**
   Endpoint URLs, bearer headers, and readiness status come from the control plane. Session prompts, session notifications, state rows, and peer ACP traffic do not.

6. **`stale` and `broken` are not-ready states.**
   A runtime that has missed its heartbeat threshold or whose provider reports failure does not satisfy rule 2's data-plane promise. Consumers must treat these the same as `starting` for the purpose of deciding whether to open `/acp` or state-stream subscriptions. This rule activates when the push lifecycle from `13b` lands; under the polling-only path it is unenforced because there is no liveness signal after `ready`.

## 5. Crate and code layout

```text
fireline/
├── crates/
│   ├── fireline-conductor/           # existing — pure library, no process
│   │   └── src/
│   │       ├── build.rs              # conductor builder
│   │       ├── trace.rs              # durable-stream tracer
│   │       ├── state_projector.rs    # state row projection
│   │       └── runtime/              # phase 0 — extracted from src/runtime_host.rs
│   │           ├── mod.rs            # RuntimeHost (public lifecycle surface)
│   │           ├── manager.rs        # RuntimeManager (internal provider dispatch)
│   │           ├── provider.rs       # RuntimeProvider trait
│   │           ├── local.rs          # LocalProvider
│   │           └── docker.rs         # DockerProvider (slice 13)
│   │
│   ├── fireline-components/          # from PR #1 — pure library
│   │   └── src/{peer, audit, context, approval, budget, smithery}.rs
│   │
│   └── fireline-control-plane/       # slice 13
│       └── src/
│           ├── main.rs               # thin axum main
│           ├── router.rs             # /v1/runtimes, /v1/auth, /v1/components
│           ├── registry.rs           # in-memory RuntimeRecord catalog
│           ├── auth.rs               # bearer token mint + validate
│           ├── heartbeat.rs          # 5s cadence, 30s stale
│           └── handlers/             # per-endpoint handlers
│
├── src/                              # the runtime binary (existing `fireline`)
│   ├── main.rs                       # unchanged entry, reads expanded env vars
│   ├── bootstrap.rs                  # accepts external DS URL, CP URL, tokens
│   ├── routes/
│   │   ├── acp.rs                    # data plane ACP WS
│   │   └── files.rs                  # data plane helper API
│   ├── control_plane_client.rs       # phase 0 / slice 13 — register + heartbeat
│   └── bin/...                       # unchanged
│
└── (removed)  src/runtime_host.rs    # moved into fireline-conductor in phase 0
```

### The key move: `RuntimeHost` into `fireline-conductor`

Today `src/runtime_host.rs` lives in the binary crate. That means it can only be used by the `fireline` runtime binary itself. After phase 0, it lives in the conductor library crate, and is embedded by **both**:

1. The `fireline-control-plane` binary — which uses `RuntimeHost` in-process to manage runtimes that the control plane spawns (via `LocalProvider` subprocesses, `DockerProvider` containers, etc.).
2. The `@durable-acp/client`'s direct-adapter mode — which embeds `RuntimeHost` via FFI or an in-process bridge for dev workflows that want "just launch me a runtime locally without standing up a control plane."

**Same type, two deployments.** The `RuntimeHost` is a library primitive. The control plane is a process that exposes it over HTTP. The dev direct adapter is a consumer that uses it in-process. Neither *is* `RuntimeHost` — both *embed* `RuntimeHost`.

### PR #1 alignment

The `fireline-components::peer::directory::Directory` introduced in [PR #1](https://github.com/gurdasnijor/fireline/pull/1) stays in place. Phase 0 renames it to `LocalPeerDirectory` and introduces a broader `PeerRegistry` trait; the new trait has:

- `LocalPeerDirectory` — existing file-backed impl, for dev mode
- `ControlPlaneRegistry` — new impl in `fireline-control-plane`, talks over HTTP to `GET /v1/runtimes`

Both implement the same `PeerRegistry` trait. `PeerComponent` takes `Arc<dyn PeerRegistry>` instead of a concrete `Directory`. Zero behavior change in dev mode.

## 6. Dev mode vs control-plane mode

**Mode A — embedded dev mode** (default, preserves today's UX):

```sh
cargo run -p fireline -- --port 4437 <agent args>
```

The runtime binary embeds a `RuntimeHost` and a `durable-streams-server` when no external URLs are configured. The control plane is absent. `client.host` uses the direct adapter, not the control-plane adapter. Single process, fast iteration. **No existing dev workflow breaks.**

**Mode B — control-plane mode** (opt-in via env vars):

```sh
# terminal 1:
cargo run -p fireline-control-plane -- --port 4440

# terminal 2 (or spawned automatically by the control plane via LocalProvider):
FIRELINE_CONTROL_PLANE_URL=http://localhost:4440 \
FIRELINE_EXTERNAL_STATE_STREAM_URL=https://ds.example.com \
  cargo run -p fireline -- ...
```

The control plane runs as a separate process. It spawns runtimes via its providers. The runtimes connect out to an external durable-streams deployment. `client.host` uses the control-plane adapter over HTTP.

The mode switch is entirely driven by the presence or absence of `FIRELINE_CONTROL_PLANE_URL`. No separate build, no feature flag, no two versions of the binary.

## 7. What is explicitly not part of either plane

- **Agent subprocess internals** (Claude Code, Codex, testy): these run under a runtime via `SharedTerminal`. Neither plane sees them directly. The runtime's ACP endpoint is the seam.
- **MCP protocol internals**: MCP messages travel over the ACP session. They are data plane traffic *inside* the ACP WebSocket. Neither plane has an MCP-level API surface.
- **Flamecast UI itself**: that is a client of both planes, not part of either. It talks HTTP to the control plane for discovery, and to runtimes or the DS deployment for data-plane observation.
- **Spike 5 peer lineage** (`_meta.fireline.*`): lives on the ACP `initialize` request — data plane transport. The control plane does not see or validate lineage.
- **Slice 10 shared-session bridge** and **remote child-session attach** (both deferred): when they eventually ship, they live on the data plane (direct runtime-to-runtime traffic), not on the control plane.

## 8. Phased delivery

The split above is deliberately sliceable. Three phases, each shippable on its own.

### Phase 0 — Runtime host extraction and peer registry trait (refactor only)

**Scope:**

- Move `src/runtime_host.rs` → `crates/fireline-conductor/src/runtime/{mod,manager,provider,local}.rs`
- Extract `RuntimeProvider` trait, with `LocalProvider` as the only implementation initially
- Keep `RuntimeHost` as the public lifecycle surface; its external API does not change
- Introduce `PeerRegistry` trait; rename `fireline_components::peer::directory::Directory` to `LocalPeerDirectory` and make it an impl of the trait
- Update `PeerComponent` to take `Arc<dyn PeerRegistry>`
- Update `src/bootstrap.rs`, `src/routes/acp.rs`, `src/runtime_host.rs` consumers for new import paths
- Commit this doc as `docs/runtime/control-and-data-plane.md`

**Shippable by itself:**

- Pure refactor, zero behavior change
- All existing tests pass
- No new binaries
- No new HTTP surfaces
- Unblocks phases 1 and 2 of slice 13 by establishing the extraction

**Delivers on its own:**

- A cleaner conductor crate that other consumers (the eventual control plane, the direct-adapter dev mode, future integration tests) can embed
- A peer-registry seam that makes it trivial to swap in a control-plane-backed registry later
- The committed decision doc — so future slices have one canonical reference instead of re-debating the split

### Phase 1 — `fireline-control-plane` binary and provider expansion

**Scope:**

- Create `crates/fireline-control-plane/` with the axum router, registry, auth, heartbeat modules
- Implement `POST /v1/runtimes` etc. over an embedded `RuntimeHost`
- Add `DockerProvider` as a second `RuntimeProvider` implementation
- Add runtime-side `control_plane_client.rs` that calls `/register` and `/heartbeat`
- Runtime bootstrap accepts `FIRELINE_CONTROL_PLANE_URL` and `FIRELINE_EXTERNAL_STATE_STREAM_URL` env vars
- One integration test: 1 local + 4 Docker runtimes registered with a single control plane, state visible in shared DS

**This is the bulk of slice 13's "distributed runtime fabric foundation" work.** Phase 0 is the prerequisite.

### Phase 2 — TypeScript primitive projection and end-to-end test

**Scope:**

- `@durable-acp/client` grows a control-plane adapter for `client.host.*`
- `RuntimeDescriptor` evolves from raw URL strings to `Endpoint` objects
- `client.state` grows a multi-stream observation helper (or documents explicit composition over `client.stream.openState`)
- One TS integration test: list runtimes through the control-plane adapter, open ACP against one, open state observation across several

**This closes the slice 13 loop.** With phase 2 done, a Flamecast-style client can manage a multi-runtime fabric entirely through the control plane without touching file-based adapters.

### Why phased delivery is the right shape

- **Phase 0 is mechanical** and can be reviewed as a pure refactor. Shipping it early means phases 1 and 2 don't have to argue about where `RuntimeHost` belongs.
- **Phase 1 is the substantive build** — the new binary, the providers, the HTTP API. It can take as long as it needs without blocking phase 0's cleanup value.
- **Phase 2 is the surface** — once phase 1 is stable, the TS client changes are additive.
- **Each phase is independently reviewable** and each phase's validation criteria are clear.

## 9. Open non-decisions

These are deliberately **not** pinned by this doc. They will be decided inside slice 13 implementation or later:

- **Auth token format.** Shared-secret opaque bearer vs JWT vs mTLS. The principle ("one authority, tokens travel with endpoints") is what matters. I would lean toward shared-secret opaque bearer tokens for slice 13 v1 and upgrade later.
- **Control-plane storage backend.** In-memory v1; SQLite, Postgres, or a dogfooded durable-streams projection later. Slice 13 v1 commits only to the in-memory shape.
- **Heartbeat-failure remediation.** What the control plane does when a runtime goes stale — auto-restart, alert only, mark-and-forget? Slice 13 defaults to mark-and-forget. Later slices can add policy.
- **Cross-region / multi-deployment control planes.** A single environment = a single control plane in this design. Federating control planes across regions is a future concern.
- **Control-plane HA.** Slice 13 v1 runs a single control plane process. Leader election and HA are deferred.

## 10. Three questions this should now answer definitively

1. **Is `RuntimeHost` the control plane?**
   **No.** `RuntimeHost` is a Rust library type that owns provider-backed runtime lifecycle in a single process. The control plane is a process that *embeds* `RuntimeHost` and exposes it over HTTP. In dev mode, `client.host`'s direct adapter embeds `RuntimeHost` too — same type, no HTTP hop. Two deployments of the same primitive.

2. **Does the durable-streams server belong to the control plane?**
   **No. It is the persistence tier of the data plane** — see §3's three-role framing (compute, persistence, consumers). The DS server stores and broadcasts events but does not process them; it is not a stream processor and not a materialization layer. Materialization happens in consumers (runtime-local projections, TS `StreamDB`, Flamecast), never in the persistence tier itself. The DS deployment is authenticated via tokens the control plane issues but is operationally independent — its lifecycle, scaling, replication, and replacement are all decisions that can be made without touching the control plane or any runtime.

3. **Where does auth happen?**
   **Control plane mints, data plane validates.** Every data-plane surface (ACP, helper files, durable-streams) accepts a bearer token in `Authorization:` and validates it against a known-good set — either a shared secret the control plane signs, a call back to the control plane's validation endpoint, or JWT signature verification. The implementation detail is open; the principle is "one authority."

---

If a later slice needs to re-open any of these decisions, the expectation is that it cites this doc, explains what changed, and either amends this doc or supersedes it with a successor.
