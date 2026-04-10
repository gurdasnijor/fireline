# 13: Distributed Runtime Fabric Foundation

Status: planned

## Objective

Prove that Fireline can create, discover, and observe heterogeneous runtimes
through one runtime/control-plane contract while preserving:

- ACP-native runtime endpoints
- ACP-native peer calls
- runtime-local coordination over durable state
- cross-runtime durability in one shared durable-streams deployment

This slice is the infrastructure and abstraction layer required to support a
topology such as:

- 1 runtime launched directly on a developer machine
- 4 runtimes launched on Docker on that same machine

The goal is not "multi-host in the abstract." The goal is to make that concrete
topology look like one coherent Fireline fabric.

## Example topology this slice is meant to unlock

```text
shared control plane
shared durable-streams deployment

node:laptop
  - runtime:laptop-local-1      provider=local
  - runtime:laptop-docker-1     provider=docker
  - runtime:laptop-docker-2     provider=docker
  - runtime:laptop-docker-3     provider=docker
  - runtime:laptop-docker-4     provider=docker
```

Important consequences of that model:

- the control plane discovers **runtimes**, not just nodes
- every runtime has its own ACP endpoint
- every runtime has its own durable state stream
- all state streams live in one shared durable-streams deployment
- peer calls stitch the distributed graph through lineage fields, not through a
  helper-side global session engine

Future expansion after this slice:

- `node:cloudflare`
  - `runtime:cf-1` provider=`cloudflare`
  - `runtime:cf-2` provider=`cloudflare`

Cloudflare remains explicitly deferred from the first implementation push. The
fabric defined here is intended to make that later provider an expansion rather
than a redesign.

## Why this slice comes after 12

Slice 12 makes optional runtime topology programmable.

That is the right point to define the distributed runtime fabric, because once
runtime composition is explicit, the next missing layer is the fabric that
creates, addresses, secures, and observes many runtimes across providers.

Without this slice:

- `client.host` remains local-only
- runtime discovery remains file-backed
- state ingest remains tied to the runtime's own embedded stream server
- multi-provider runtime placement remains an implementation detail with no
  stable contract

## What this slice should prove

- `client.host` can target a control-plane-backed runtime API rather than only a
  local child-process launcher.
- Fireline has a provider-neutral runtime manager boundary in Rust.
- runtime discovery is runtime-centric and control-plane-backed, not file-based
  and node-centric.
- runtime bootstrap supports:
  - explicit `nodeId`
  - explicit advertised ACP endpoint
  - explicit external durable-streams endpoint
- Fireline runtimes can all write `STATE-PROTOCOL` rows into one shared
  durable-streams deployment while keeping per-runtime streams.
- runtime-local materializers continue to read only their own stream.
- TypeScript consumers can observe many runtimes by opening many streams from
  the same durable-streams deployment.
- lineage and state joins remain reconstructible from persisted trace alone.

## Scope

### 1. Control-plane runtime API

Add a control-plane-backed runtime lifecycle surface that implements the same
primitive contract already described in [`../ts/primitives.md`](../ts/primitives.md):

- `create`
- `get`
- `list`
- `stop`
- `delete`

Recommended HTTP shape:

- `POST /v1/runtimes`
- `GET /v1/runtimes`
- `GET /v1/runtimes/{runtimeKey}`
- `POST /v1/runtimes/{runtimeKey}/stop`
- `DELETE /v1/runtimes/{runtimeKey}`

The control plane becomes the canonical bootstrap/discovery surface for remote
and multi-provider runtimes.

Recommended packaging for this slice:

- add a new control-plane binary, `fireline-control-plane`
- implement it as an axum server with its own router and auth boundary
- reuse the same runtime descriptor and runtime host/provider types as the rest
  of Fireline rather than inventing a second model

This keeps the control plane explicit in deployment and testing:

- Fireline runtime processes remain runtimes
- the control plane remains a distinct process
- the end-to-end tests can bring both up deliberately

### 2. Runtime manager and provider boundary

Introduce a provider-neutral Rust boundary that owns imperative runtime
lifecycle.

Recommended shape:

- keep `RuntimeHost` as the public lifecycle surface
- extract a provider-backed runtime-manager layer from the existing
  `RuntimeHost` implementation rather than adding a second competing abstraction
- add a `RuntimeProvider` trait behind that surface
- add provider-specific implementations behind that trait

Expected providers:

- `LocalProvider`
- `DockerProvider`

The point of this slice is not to implement every provider fully. The point is
to establish the common runtime fabric they all plug into.

Provider expansion explicitly deferred past this slice:

- `CloudflareProvider`

Implementation note:

- `src/runtime_host.rs` already has the right public shape
- slice 13 should treat this as a refactor/extraction, not as a net-new public
  API

### 3. Runtime bootstrap contract

Refactor bootstrap so runtime bring-up no longer assumes:

- bind address == advertised address
- embedded durable-streams server == canonical state destination
- host-derived `nodeId`
- local file registries for discovery

The runtime should instead accept:

- `runtimeKey`
- `nodeId`
- advertised ACP endpoint
- external state-stream endpoint
- control-plane registration endpoint
- registration/auth credentials

### 4. Runtime registration and heartbeat

Each runtime should self-register with the control plane after boot and publish:

- `runtimeKey`
- `runtimeId`
- `nodeId`
- provider kind
- provider instance id
- advertised ACP endpoint
- state stream endpoint
- health / readiness status

The control plane should treat registration plus successful health as the
transition to `ready`.

Recommended defaults for this slice:

- heartbeat period: 5 seconds
- stale timeout: 30 seconds without heartbeat
- `broken` / `stale` records remain queryable until explicit stop/delete
- automatic record deletion is out of scope for the first cut

The point is not to perfect runtime liveness semantics in this slice. The point
is to avoid ghost runtimes and make failure modes observable from day one.

### 5. Shared durable-streams deployment

This slice adopts a specific deployment model:

- one durable-streams deployment per environment
- one state stream per runtime inside that deployment
- runtime-local materializers read only their own stream
- global observers read many runtime streams from the same deployment

This is intentionally **not**:

- one embedded durable-streams server per runtime as the only production model
- one giant global stream for every runtime in the environment

The shared durable-streams service becomes the durability substrate; the runtime
may still embed durable-streams in local-only mode.

### 6. TypeScript primitive projection

Update the TS primitive surface so distributed runtimes project cleanly.

Recommended changes:

#### `RuntimeDescriptor`

Evolve from raw URLs to endpoint objects:

```ts
type Endpoint = {
  url: string;
  headers?: Record<string, string>;
};

type RuntimeDescriptor = {
  runtimeKey: string;
  runtimeId: string;
  nodeId: string;
  provider: "local" | "docker" | "cloudflare";
  providerInstanceId: string;
  status: "starting" | "ready" | "busy" | "idle" | "stale" | "broken" | "stopped";
  acp: Endpoint;
  state: Endpoint;
  helperApiBaseUrl?: string;
  createdAtMs: number;
  updatedAtMs: number;
};
```

That lets:

- `client.acp.connect(runtime.acp)`
- `client.stream.openState(runtime.state)`

work without out-of-band token plumbing.

#### `client.host`

Support two implementation modes behind the same primitive:

- direct/local adapter
- control-plane HTTP adapter

The primitive contract stays the same; only the transport differs.

The control-plane `POST /v1/runtimes` request body should carry the same
`CreateRuntimeSpec` shape as the local adapter, including:

- provider selection
- agent launch spec
- optional `topology: TopologySpec`

That keeps slice 12 and slice 13 composable instead of creating separate local
and remote runtime-creation models.

#### `client.peer`

Peer discovery should become runtime-centric.

The existing peer descriptor is too node-shaped for multi-runtime-per-node
topologies. The peer descriptor should identify the target runtime explicitly.

#### `client.state`

This slice should define a multi-stream observation path for distributed views.

Acceptable options:

- `client.state.openMany({ streams })`
- or explicit composition by opening many `client.stream` handles

The important point is that multi-runtime observation becomes an explicit
primitive concern rather than an ad hoc control-plane snapshot.

### 7. Discovery model

Replace the current file-backed discovery story with a control-plane-backed
runtime catalog.

Required distinctions:

- `nodeId` identifies the hosting domain
- `runtimeKey` identifies the stable runtime record
- `runtimeId` identifies the concrete runtime instance

One node may host many runtimes.

That is essential for the example topology where a single machine hosts:

- 1 directly launched runtime
- 4 Docker-launched runtimes

The existing file-backed peer directory should remain only as a local-dev
adapter.

Expected direction:

- keep the current local file implementation as `LocalPeerDirectory` or similar
- introduce a broader runtime/peer registry trait
- use the control plane as the distributed implementation of that trait

### 8. Auth model

This slice should define auth on three surfaces:

- control-plane runtime API
- ACP runtime endpoints
- durable-streams read/write endpoints

Bearer tokens are the simplest first target.

The important design point is that endpoint auth travels inside the returned
endpoint objects rather than through hidden local config.

## Shared durable-streams model

This is the key deployment and data-flow decision.

### Desired shape

All runtimes append durable state changes into one shared durable-streams
deployment, but not into one shared stream.

Recommended model:

- stream naming derived from `runtimeKey`
- one stream per runtime
- same durable-streams cluster/service for all streams in the environment

Example:

- `https://ds.example.com/v1/stream/fireline-state-runtime-a`
- `https://ds.example.com/v1/stream/fireline-state-runtime-b`
- `https://ds.example.com/v1/stream/fireline-state-runtime-c`

### Why per-runtime streams inside one deployment

This preserves the current Fireline runtime-local materializer model cleanly:

- runtime-local projections subscribe to one stream
- replay boundaries stay simple
- operational isolation is clearer
- cross-runtime consumers still read from one shared data store

### Why not one global stream in this slice

One global stream introduces more work immediately:

- global ordering questions
- replay volume for runtime-local materializers
- noisier operational boundaries
- more complicated filtering in every runtime

A global aggregate stream may be valuable later, but it should be additive.

### Implication for `trace.rs`

`crates/fireline-conductor/src/trace.rs` does not need a new tracing model.

It already writes through a `durable_streams::Producer`.

The required change is in bootstrap/config:

- the producer should be allowed to point at an external durable-streams
  service
- the runtime materializer should read from that same external service
- embedded durable-streams should become optional rather than mandatory

## Data model implications

Cross-runtime joins are already structurally compatible with a shared
durable-streams deployment because projected state rows carry runtime identity.

Examples already projected into durable state include:

- `runtimeKey`
- `runtimeId`
- `nodeId`
- `logicalConnectionId`
- lineage fields such as `traceId` and `parentPromptTurnId`

That means Flamecast or another consumer can reconstruct the distributed graph
by reading many runtime streams and joining on those durable fields rather than
depending on host-local registries.

## Proposed architecture

```text
TS client
  -> client.host (control-plane adapter)
     -> control-plane runtime API
        -> runtime manager on node:laptop
           -> LocalProvider / DockerProvider

runtime
  -> ACP endpoint
  -> writes state through trace.rs producer
  -> shared durable-streams deployment
  -> runtime-local materializer reads its own stream

observer
  -> control-plane list of runtimes
  -> opens many runtime streams from same durable-streams deployment
  -> materializes cross-runtime view in TypeScript
```

## Acceptance criteria

- a control-plane-backed `client.host` adapter exists
- the runtime/control-plane API can return runtime descriptors for a mixed
  topology of local and Docker-backed runtimes
- Fireline bootstrap can be configured with:
  - explicit `nodeId`
  - explicit advertised ACP endpoint
  - external durable-streams endpoint
- embedded durable-streams remains available for local-only mode but is no
  longer the only deployment shape
- one end-to-end topology can be exercised with:
  - 1 local runtime
  - 4 Docker runtimes
  - one shared durable-streams deployment
- all exercised runtimes write state rows to the shared durable-streams
  deployment using distinct per-runtime streams
- a TypeScript consumer can observe those runtimes by opening multiple streams
  from the same durable-streams deployment
- one cross-runtime peer call proves lineage and durable joins still work across
  that shared deployment

## Validation

- `cargo test -q`
- `pnpm --filter @fireline/client test`
- one new distributed-fabric integration test that:
  - starts a shared durable-streams service
  - launches one local runtime and multiple Docker runtimes against it
  - verifies each runtime registers and exposes a descriptor
  - verifies each runtime writes to its own stream in the shared deployment
  - verifies a cross-runtime peer call can be reconstructed from durable state
- one TS integration test that:
  - lists runtimes through `client.host`
  - opens ACP against one returned runtime endpoint
  - opens state observation across multiple returned state endpoints

## Deferred

- actual Cloudflare provider bring-up and packaging details
- one global aggregate stream across all runtimes
- automatic failover / migration of live sessions across runtimes
- cross-region replication of the durable-streams deployment
- shared-session bridge semantics
- per-session runtime placement
- a richer control-plane query language over runtime topology

## Why this slice is the right boundary

This slice deliberately centralizes the common fabric once:

- runtime API
- provider boundary
- registration
- discovery
- auth
- shared durable-streams deployment model
- TS primitive projection

That way provider-specific work such as Cloudflare packaging does not become a
second architecture.

After this slice, adding:

- `CloudflareProvider`
- `KubernetesProvider`
- heavier sandbox providers

should be a provider expansion, not a redesign of Fireline's runtime and
durability model.
