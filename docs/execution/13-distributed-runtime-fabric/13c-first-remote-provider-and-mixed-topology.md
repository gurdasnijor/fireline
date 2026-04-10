# 13c: First Remote Provider and Mixed Topology

Status: planned
Type: execution slice

Related:

- [`./README.md`](./README.md)
- [`./phase-0-runtime-host-and-peer-registry-refactor.md`](./phase-0-runtime-host-and-peer-registry-refactor.md)
- [`./13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md`](./13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md)
- [`./13b-push-lifecycle-and-auth.md`](./13b-push-lifecycle-and-auth.md)
- [`../../runtime/control-and-data-plane.md`](../../runtime/control-and-data-plane.md)
- [`../../runtime/heartbeat-and-registration.md`](../../runtime/heartbeat-and-registration.md)

## Objective

Prove that the runtime contract from `13a` plus the push lifecycle from `13b`
extends cleanly to a true mixed topology:

- one local runtime
- multiple runtimes from a non-local provider
- one shared durable-streams deployment

This is the slice where non-local provider assumptions stop being
hypothetical.

## Product Pillar

Provider-neutral runtime fabric.

## User Workflow Unlocked

Observe and manage one coherent local + remote runtime fabric without
changing the logical runtime contract or the durable session story.

## Why This Comes After `13b`

`13a` proved the contract.

`13b` proved the contract works under push instead of polling, and put auth
on the new write surface. After `13b`, the lifecycle protocol is
provider-agnostic — the same `/register` and `/heartbeat` flow that the local
launcher exercises is the flow any remote provider must use.

`13c` proves that contract survives once:

- runtimes no longer share a filesystem with the control plane
- runtime registration cannot rely on local polling assumptions
- a single node hosts heterogeneous providers
- one cross-runtime peer call has to traverse the mixed topology

If `13c` lands first or in parallel with `13b`, every failure becomes
ambiguous: was it the protocol, the provider, the auth, the topology, or the
peer call? Sequencing them keeps the review surface localized.

## Choice of First Remote Provider

This slice ships with **one** remote provider. The architecture supports
several candidates:

- **Docker** via `bollard` — already named in the umbrella, no third-party
  service dependency in CI, can use bind mounts as an escape hatch (which is
  partly the reason to *not* use them — they would mask network-only
  failures we want to find now)
- **E2B** via the Node SDK or a Rust client — Firecracker microVMs with
  `getHost(port)` for public URLs, strictest network-only test, third-party
  account required
- **Daytona** via `daytona-client` — workspace model with preview URLs,
  similar shape to E2B

The choice belongs to whoever picks up this slice. The acceptance criteria
below are written against "the first remote provider" without naming one,
because the contract is provider-agnostic by design. The handoff prompt
should specify which.

Recommendation if undecided: pick the strictest network-only candidate,
because passing acceptance against it implies passing against the others.
That is currently E2B.

## Scope

### 1. First remote `RuntimeProvider`

Implement one `RuntimeProvider` impl backed by a non-local launcher. It must:

- create runtimes through the `RuntimeProvider::start()` boundary defined in
  `crates/fireline-conductor/src/runtime/provider.rs`
- always set `--control-plane-url` and `FIRELINE_CONTROL_PLANE_TOKEN` on the
  spawned runtime — polling mode is not an option for non-local launchers
  and the launcher should fail fast with a clear error if it ever tries to
  spawn without a control-plane URL
- compute the runtime's `provider_instance_id` (container id, sandbox id,
  workspace id) at spawn time and surface it on the descriptor when the
  runtime registers
- compute the runtime's *actual* advertised ACP URL and durable-stream URL
  after launch — these can differ from any pre-launch guess (port mappings,
  proxy URLs, signed preview URLs) and must be sourced from the launcher
  output, not from a hardcoded template

### 2. Mixed topology discovery

The control plane must be able to return descriptors for a mixed topology:

- 1 local runtime
- 4 runtimes from the new remote provider

Discovery is runtime-centric, not node-centric. `GET /v1/runtimes` returns
all five descriptors uniformly; `GET /v1/runtimes/{key}` works against any of
them; `POST /v1/runtimes/{key}/stop` and `DELETE /v1/runtimes/{key}` route
through the right provider.

### 3. Multi-stream observation in TS

TypeScript consumers must be able to observe several runtime streams from
the same shared durable-streams deployment.

Acceptable shape:

- an explicit `openMany` helper on `client.state`
- or explicit composition over many `openState` handles

The important property is that distributed observation becomes a first-class
consumption path, not an ad-hoc loop.

### 4. Cross-runtime peer call proof

One end-to-end test where:

- runtime A hosts a session
- runtime A calls `prompt_peer` against runtime B
- the peer call traverses the mixed topology (one local, one remote)
- lineage is reconstructible from durable state alone, by reading both
  runtimes' streams from the shared DS deployment and stitching by
  `traceId` / `parentPromptTurnId`

### 5. Endpoint object completeness

If the migration started in `572dd0b` left any raw URL strings in the
runtime descriptor surface or its consumers, finish the migration here.
`13c` is the first slice that exercises descriptors against a non-local
runtime, which is when string-vs-Endpoint mismatches surface as bugs.

## Explicit Non-Goals

This slice does **not** add:

- A second remote provider
- A control-plane lifecycle event stream or `RuntimeRegistryProjector`
- A global aggregate stream across all runtimes
- Automatic live-session migration across runtimes
- Cross-region durable-streams replication
- Cloudflare or Kubernetes provider work
- Richer scheduling or placement policy
- Changes to the runtime contract introduced in `13a` or the push lifecycle
  introduced in `13b`

## Files Likely Touched

Rust:

- `crates/fireline-conductor/src/runtime/<provider>.rs` — new provider impl
- `crates/fireline-conductor/src/runtime/provider.rs` — additional fields
  on `RuntimeProvider` if needed for the new launcher
- `crates/fireline-control-plane/src/main.rs` — registration of the new
  provider variant
- `tests/distributed_mixed_topology.rs` — new integration test

TypeScript:

- `packages/client/src/state.ts` — `openMany` helper or composition utility
- tests that observe multi-runtime streams

## Acceptance Criteria

- The chosen first remote `RuntimeProvider` can create and stop Fireline
  runtimes through the same control-plane API as `LocalProvider`
- Remote runtimes register via the `13b` push surface; polling is not
  attempted
- The remote provider passes through `provider_instance_id` and the
  runtime's actual advertised ACP / state-stream URLs end-to-end, and
  `RuntimeDescriptor` reflects them
- Bearer auth from `13b` covers remote runtimes — a token issued for a
  remote runtime is scoped to its own `runtime_key` and cannot register or
  heartbeat for any other runtime, local or remote
- The control plane returns descriptors for:
  - 1 local runtime
  - 4 remote runtimes
- All five runtimes write durable state into one shared durable-streams
  deployment using distinct per-runtime streams
- A TypeScript consumer can observe multiple runtime streams from that
  shared deployment in one cohesive call shape
- One cross-runtime peer call traversing the mixed topology can be
  reconstructed from durable state alone

## Validation

- `cargo test -q`
- `pnpm --filter @fireline/client test`
- one distributed integration test that:
  - starts one control plane
  - starts one shared durable-streams service
  - launches 1 local runtime and 4 remote runtimes
  - verifies push registration and `ready` for all runtimes
  - verifies bearer auth rejection across runtime boundaries
  - verifies each runtime writes to its own per-runtime stream
  - verifies one peer call across runtimes preserves lineage
- one TS integration test that:
  - lists runtimes through the control-plane-backed `client.host`
  - waits for `ready` on all of them
  - opens observation across multiple returned state endpoints

## Handoff Note

This slice is appropriate for Codex only after both:

- `phase-0` is done
- `13b` is in place

Without `13b`, the implementation has to either invent a one-off readiness
mechanism for the remote provider or hand-roll push code inside the provider
crate. Both paths produce code that has to be ripped out when `13b` lands
properly. Wait.

The handoff prompt should pick the first remote provider explicitly.
