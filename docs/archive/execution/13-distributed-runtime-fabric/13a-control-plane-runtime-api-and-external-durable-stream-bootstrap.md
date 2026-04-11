# 13a: Control-Plane Runtime API and External Durable-Stream Bootstrap

Status: planned
Type: execution slice

Related:

- [`./README.md`](./README.md)
- [`./phase-0-runtime-host-and-peer-registry-refactor.md`](./phase-0-runtime-host-and-peer-registry-refactor.md)
- [`../../runtime/control-and-data-plane.md`](../../runtime/control-and-data-plane.md)
- [`../../ts/primitives.md`](../../ts/primitives.md)
- [`../../product/priorities.md`](../../product/priorities.md)
- [`../../product/runs-and-sessions.md`](../../product/runs-and-sessions.md)

## Objective

Prove the first environment-level runtime contract:

- a control plane exists
- `client.host` can target it
- runtime descriptors come from it
- durable state can live outside the runtime process

This first cut should stay intentionally narrow:

- `LocalProvider` only
- no Docker yet
- no mixed-provider topology yet

## Product Pillar

Provider-neutral runtime fabric.

## User Workflow Unlocked

Start, list, and observe a Fireline runtime through a control-plane-backed
contract, with durable state stored in an external durable-streams deployment
rather than only in an embedded per-runtime server.

## Why This Slice Exists

Today Fireline is still fundamentally local-process oriented:

- `client.host` launches local child processes
- runtime discovery is tied to local registries
- bootstrap derives data-plane endpoints from the runtime's own listener
- embedded durable-streams is the default durable destination

That makes later provider work much harder because provider expansion would also
have to redefine the runtime contract.

`13a` fixes the contract first.

## Scope

### 1. Control-plane binary

Add a distinct control-plane process:

- `fireline-control-plane`

It should expose a runtime lifecycle HTTP API over the extracted runtime host
surface.

Required endpoints:

- `POST /v1/runtimes`
- `GET /v1/runtimes`
- `GET /v1/runtimes/{runtimeKey}`
- `POST /v1/runtimes/{runtimeKey}/stop`
- `DELETE /v1/runtimes/{runtimeKey}`

This slice may keep readiness/local-process coordination simple because the only
provider is still `LocalProvider`.

### 2. External durable-stream bootstrap

Refactor runtime bootstrap so the runtime can write durable state to an
external durable-streams deployment instead of only to an embedded server.

Required bootstrap distinctions:

- bind address vs advertised ACP endpoint
- embedded durable-streams vs external durable-streams endpoint
- explicit `runtimeKey`
- explicit `nodeId`

Embedded durable-streams should remain available for local-only mode, but it
must stop being the only valid deployment shape.

### 3. Runtime descriptor endpoint objects

Move the descriptor surface from raw URL strings to endpoint objects:

```ts
type Endpoint = {
  url: string;
  headers?: Record<string, string>;
};
```

This allows auth to travel with the descriptor rather than through hidden local
config.

### 4. `client.host` control-plane adapter

Add a control-plane-backed implementation mode for `client.host` behind the same
primitive contract.

This slice should preserve the existing local/direct adapter while adding a
control-plane adapter as a second mode.

### 5. Readiness discipline

Clients must not probe ACP or stream routes speculatively.

This slice should make the contract explicit:

- read the runtime descriptor first
- wait for `status: "ready"`
- then open ACP or state observation

Dev-mode proxies are transport conveniences, not readiness signals.

## Explicit Non-Goals

This slice does **not** add:

- `DockerProvider`
- push-based registration and heartbeat for non-local providers
- mixed local + Docker topology proof
- runtime-centric distributed peer registry
- Cloudflare provider work

Those belong to later steps in the umbrella.

## Files Likely Touched

Rust:

- `crates/fireline-control-plane/src/main.rs`
- `crates/fireline-control-plane/src/router.rs`
- `crates/fireline-control-plane/src/registry.rs`
- `src/bootstrap.rs`
- `src/main.rs`

TypeScript:

- `packages/client/src/host.ts`
- `packages/client/src/index.ts`
- tests that consume runtime descriptors or `client.host`

## Acceptance Criteria

- a control-plane-backed `client.host` adapter exists
- the control plane can create, list, get, stop, and delete runtimes through
  `LocalProvider`
- a runtime can be started with:
  - explicit `runtimeKey`
  - explicit `nodeId`
  - an advertised ACP endpoint
  - an external durable-streams endpoint
- embedded durable-streams remains available for local-only mode
- `RuntimeDescriptor` uses endpoint objects rather than raw URL strings
- a TypeScript client reads `status: "ready"` from the descriptor before
  touching ACP or durable-streams routes
- one runtime can write `STATE-PROTOCOL` rows to an external durable-streams
  deployment and read them back into its runtime-local materializer

## Validation

- `cargo test -q`
- `pnpm --filter @fireline/client test`
- one integration test that:
  - starts the control plane
  - starts one local runtime through the control plane
  - points that runtime at an external durable-streams service
  - verifies the runtime becomes `ready`
  - verifies state is written to and replayed from the external stream
- one TS integration test that:
  - creates a runtime through the control-plane-backed `client.host`
  - waits for `status: "ready"`
  - then connects ACP and state observation through the descriptor endpoints

## Handoff Note

This is the first slice in the 13 umbrella that should be handed to Codex as a
feature implementation task.

The handoff should emphasize:

- `LocalProvider` only
- external durable-streams support is required
- keep Docker and push heartbeat work out of scope
