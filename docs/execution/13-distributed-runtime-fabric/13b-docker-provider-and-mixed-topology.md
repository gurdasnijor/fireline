# 13b: Docker Provider and Mixed Topology

Status: planned
Type: execution slice

Related:

- [`./README.md`](./README.md)
- [`./phase-0-runtime-host-and-peer-registry-refactor.md`](./phase-0-runtime-host-and-peer-registry-refactor.md)
- [`./13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md`](./13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md)
- [`../../runtime/control-and-data-plane.md`](../../runtime/control-and-data-plane.md)
- [`../../runtime/heartbeat-and-registration.md`](../../runtime/heartbeat-and-registration.md)
- [`../../product/priorities.md`](../../product/priorities.md)

## Objective

Prove that the runtime fabric defined in `13a` extends cleanly to a true mixed
topology:

- one local runtime
- multiple Docker-backed runtimes
- one shared durable-streams deployment

This is the slice where non-local provider assumptions stop being hypothetical.

## Product Pillar

Provider-neutral runtime fabric.

## User Workflow Unlocked

Observe and manage one coherent local + Docker runtime fabric without changing
the logical runtime contract or the durable session story.

## Why This Comes After `13a`

`13a` proves the contract.

`13b` proves that the contract survives once:

- runtimes no longer share a filesystem with the control plane
- runtime registration cannot rely on local polling assumptions
- a single node hosts heterogeneous providers

## Scope

### 1. `DockerProvider`

Add `DockerProvider` on top of the extracted `RuntimeProvider` boundary.

This slice should prove:

- containerized runtime launch
- provider instance identity
- lifecycle operations through the same control-plane contract

### 2. Push-based registration and heartbeat

Docker-backed runtimes must not rely on shared local files for readiness or
liveness.

This slice should adopt the push-based model described in
[`../../runtime/heartbeat-and-registration.md`](../../runtime/heartbeat-and-registration.md):

- runtime calls `/register`
- runtime calls `/heartbeat`
- control plane tracks `ready`, `stale`, `broken`, and `stopped`

### 3. Mixed topology discovery

The control plane should be able to return runtime descriptors for a mixed
topology of:

- one `local` runtime
- multiple `docker` runtimes

Discovery must be runtime-centric, not node-centric.

### 4. Multi-stream observation

TypeScript consumers should be able to observe several runtime streams from the
same durable-streams deployment.

Acceptable outcome:

- an explicit `openMany` helper
- or explicit composition over many stream handles

The important part is that distributed observation becomes a first-class
consumption path.

### 5. Cross-runtime proof

This slice should include one cross-runtime peer call over the mixed topology,
with durable lineage reconstructible from the shared durable-streams
deployment.

## Explicit Non-Goals

This slice does **not** add:

- Cloudflare provider
- Kubernetes or other scheduler-backed providers
- a global aggregate stream
- automatic live-session migration
- richer scheduling or placement policy

## Acceptance Criteria

- `DockerProvider` can create and stop Fireline runtimes through the control
  plane
- Docker-backed runtimes register and heartbeat without shared filesystem
  assumptions
- the control plane can return descriptors for:
  - 1 local runtime
  - 4 Docker runtimes
- all runtimes write durable state into one shared durable-streams deployment
  using distinct per-runtime streams
- a TypeScript consumer can observe multiple runtime streams from that shared
  deployment
- one cross-runtime peer call can be reconstructed from durable state across
  that mixed topology

## Validation

- `cargo test -q`
- `pnpm --filter @fireline/client test`
- one distributed integration test that:
  - starts one control plane
  - starts one shared durable-streams service
  - launches 1 local runtime and 4 Docker runtimes
  - verifies registration and readiness for all runtimes
  - verifies each runtime writes to its own stream
  - verifies one peer call across runtimes preserves lineage
- one TS integration test that:
  - lists runtimes through the control-plane-backed client
  - waits for `ready`
  - opens observation across multiple returned state endpoints

## Handoff Note

This slice is appropriate for Codex only after:

- phase 0 is done
- `13a` is in place

Without those prerequisites, the implementation risk is too high and the review
surface is too large.
