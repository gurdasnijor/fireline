# 13: Distributed Runtime Fabric

Status: planned

Related:

- [`../../runtime/control-and-data-plane.md`](../../runtime/control-and-data-plane.md)
- [`../../runtime/heartbeat-and-registration.md`](../../runtime/heartbeat-and-registration.md)
- [`../../ts/primitives.md`](../../ts/primitives.md)
- [`../../product/vision.md`](../../product/vision.md)
- [`../../product/object-model.md`](../../product/object-model.md)
- [`../../product/roadmap-alignment.md`](../../product/roadmap-alignment.md)
- [`../12-programmable-topology-first-mover.md`](../12-programmable-topology-first-mover.md)

## Purpose

This folder replaces the old single-doc slice 13 with a structure that is small
enough to hand off to Codex without ambiguity.

The old `13-distributed-runtime-fabric-foundation.md` captured the architecture
correctly, but it mixed:

- prerequisite refactor work
- first control-plane/runtime contract work
- Docker and mixed-topology expansion
- TypeScript surface projection

That was too large for one implementation prompt or one review cycle.

This folder keeps slice 13 as the umbrella and splits it into reviewable,
Codex-sized units.

## Objective

Prove that Fireline can create, discover, and observe heterogeneous runtimes
through one runtime/control-plane contract while preserving:

- ACP-native runtime endpoints
- ACP-native peer calls
- runtime-local coordination over durable state
- cross-runtime durability in one shared durable-streams deployment

## Product Pillar

Provider-neutral runtime fabric.

## User Workflow Unlocked

Start, discover, and observe one logical fleet of Fireline runtimes across
local and later Docker-backed placements while preserving durable sessions and
ACP-native peer behavior.

## Why This Is Split

The important boundary is:

- this folder is the architectural umbrella
- the child docs are the actual handoff targets

Do not hand the whole umbrella to Codex as one implementation request.

Start from one of the child docs below.

## Delivery Sequence

### 1. Phase 0 — prerequisite refactor

[`phase-0-runtime-host-and-peer-registry-refactor.md`](./phase-0-runtime-host-and-peer-registry-refactor.md)

Purpose:

- extract the runtime/provider seam
- extract the peer registry seam
- preserve current behavior

This is a pure refactor, not a product-visible slice.

### 2. `13a` — control-plane API and external durable-stream bootstrap

[`13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md`](./13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md)

Purpose:

- prove the environment-level runtime contract
- keep scope to `LocalProvider`
- allow durable state to live outside the runtime

This is the first real Codex handoff target.

### 3. `13b` — push lifecycle and auth (LocalProvider only)

[`13b-push-lifecycle-and-auth.md`](./13b-push-lifecycle-and-auth.md)

Purpose:

- gate the readiness flip in `RuntimeHost::create()` so push can drive it
- add `POST /v1/runtimes/{key}/register` and `POST /v1/runtimes/{key}/heartbeat`
- add the heartbeat tracker and `starting → ready → stale ↔ ready → stopping
  → stopped` state machine, plus `broken`
- add bearer auth via `POST /v1/auth/runtime-token` and middleware
- add the runtime-side push client and `LocalProvider::prefer_push: bool`

Stays on `LocalProvider` only. Polling remains the default; push is opt-in.

### 4. `13c` — first remote provider and mixed-topology proof

[`13c-first-remote-provider-and-mixed-topology.md`](./13c-first-remote-provider-and-mixed-topology.md)

Purpose:

- add the first non-local `RuntimeProvider` (Docker, E2B, or Daytona —
  decided at handoff time)
- prove one mixed local + remote topology against a shared durable-streams
  deployment
- add multi-stream observation in TS clients
- prove one cross-runtime peer call traverses the mixed topology with
  reconstructible lineage

Depends on `13b`. Without push lifecycle and auth in place, this slice would
have to invent its own readiness signaling per provider and the runtime
contract would silently bifurcate.

## Example Topology The Full Slice Should Unlock

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

Important consequences:

- the control plane discovers runtimes, not just nodes
- every runtime has its own ACP endpoint
- every runtime has its own durable state stream
- all runtime streams live in one shared durable-streams deployment
- lineage reconstruction comes from durable state, not host-local side files

## What Is Deferred From This Folder

- Cloudflare provider bring-up and packaging details
- one global aggregate stream across all runtimes
- automatic live-session migration across runtimes
- cross-region durable-streams replication
- shared-session bridge semantics
- richer control-plane scheduling or query language

## Acceptance For The Umbrella

The umbrella is done when:

- the runtime/provider seam is extracted cleanly (phase 0)
- a control-plane-backed runtime contract exists (`13a`)
- external durable-streams is a valid deployment shape (`13a`)
- the push lifecycle and auth are in place against `LocalProvider` (`13b`)
- a mixed local + remote runtime fabric can be observed coherently (`13c`)
- TypeScript clients can consume that fabric through honest descriptors

Until then, use the child docs as the actual implementation units.
