# 12: Programmable Topology First Mover

Status: planned

Related:

- [`../product/vision.md`](../product/vision.md)
- [`../product/ecosystem-story.md`](../product/ecosystem-story.md)
- [`../product/roadmap-alignment.md`](../product/roadmap-alignment.md)

## Objective

Prove that Fireline can build its optional ACP component chain from an explicit
topology specification instead of hardcoding those components inside the `/acp`
route.

This slice is intentionally narrow. It does not try to solve every possible
proxy or dynamic per-session override. It only needs to prove the architecture
with one observer, one inbound transformer, and the existing peer MCP tool
injector.

## Product Pillar

Reusable conductor extensions.

## User Workflow Unlocked

Let a run gain reusable capabilities such as audit, context injection, and peer
delegation without forking the underlying harness or baking those concerns into
one agent implementation.

## Why this slice comes next

Slice 11 closed the loop on runtime launch:

- catalog discovery exists
- local resolution exists
- `client.host.create(...)` can launch by agent reference
- the browser harness can select and launch agents without hardcoded commands

The next obvious hardcoded seam is runtime ACP composition. Today Fireline
still builds that chain directly in
[`src/routes/acp.rs`](/Users/gnijor/gurdasnijor/fireline/src/routes/acp.rs).

That is where programmable topology should start.

## What this slice should prove

- TypeScript can describe a runtime topology at runtime creation time.
- Fireline can resolve named topology components from a registry.
- Optional ACP components are no longer wired directly in the route handler.
- The first three components compose correctly:
  - `peer_mcp`
  - `audit`
  - `context_injection`
- The existing runtime/session substrate remains intact.

## Scope

### Rust

- add a `ComponentRegistry` surface in `fireline-conductor`
- add a `ComponentContext` surface for runtime-scoped dependencies
- move `PeerComponent` registration behind that registry
- add `fireline-audit` as a pure observer component
- add `fireline-context` as an inbound transformer component
- change bootstrap/route wiring so topology-selected components come from a
  `TopologySpec`

### TypeScript

- reserve and document `client.topology`
- add a builder that emits a `TopologySpec`
- allow `client.host.create({ topology })`
- keep topology creation-time only for this slice

### Runtime behavior

- `LoadCoordinatorComponent` remains a fixed system component
- user-specified topology controls optional runtime components only
- unknown component names fail explicitly
- invalid ordering fails explicitly

## Topology boundary for v1

This slice draws one important line:

- system coordination components stay fixed
- user-selected topology components are additive and replace the current
  hardcoded optional chain

Concretely, that means:

- `LoadCoordinatorComponent` is still mounted by Fireline itself
- `peer_mcp` moves into the registry because it is an optional capability
- future components like `audit` and `context_injection` live in the same
  registry-driven path

This keeps reconnect/session coordination outside the user-configurable
topology surface.

## Acceptance criteria

- `client.topology.builder()` can produce a valid `TopologySpec`
- `client.host.create({ provider: "local", agent, topology })` launches a
  runtime with that topology
- `peer_mcp` is registry-driven rather than hardcoded in the route
- `audit` writes observed traffic to its configured stream
- `context_injection` modifies inbound prompt context before the terminal agent
  receives it
- one end-to-end test proves all three components on a real runtime

## Validation

- `cargo test -q`
- `pnpm --filter @fireline/client test`
- one new end-to-end topology test that:
  - builds a topology in TS
  - launches a runtime with it
  - verifies audit output
  - verifies injected context is visible to the downstream agent

## Deferred

- approval gates
- budget caps
- recording / replay
- initialize-time topology negotiation
- per-session topology overrides
- server-published component schemas
- distributed-runtime layout
- ACP shared-session bridge semantics
