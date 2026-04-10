# Next Steps Decision

> Status: adopted direction
> Type: decision log
> Audience: maintainers continuing after slice 11
> Related:
> - [`11-agent-catalog-and-runtime-launch.md`](./11-agent-catalog-and-runtime-launch.md)
> - [`12-programmable-topology-first-mover.md`](./12-programmable-topology-first-mover.md)
> - [`10-acp-shared-session-bridge.md`](./10-acp-shared-session-bridge.md)
> - [`../programmable-topology-exploration.md`](../programmable-topology-exploration.md)

## Purpose

Record the decision that followed the old "remote child-session attach" idea.

That idea was considered as a possible next slice after the session-durability
work. It is now explicitly deferred. Slice 11 shipped as agent catalog and
runtime launch instead, and the next shipping slice is slice 12:
programmable topology first mover.

## Current state

Shipped:

- slice 01 through slice 09
- slice 11 agent catalog and runtime launch

Deferred:

- slice 10 ACP shared-session bridge
- remote child-session attach as a standalone slice

Planned next:

- slice 12 programmable topology first mover

## Why remote child-session attach was deferred

The old proposal was internally consistent:

- slice 08 proved same-runtime `session/load`
- slice 09 proved durable `child_session_edge`
- the next obvious composition was cross-node child-session attach

The problem was consumer pull. Nothing in the repo currently needs to navigate
the distributed session graph:

- the browser harness is using catalog-driven runtime launch, not graph
  navigation
- no control-plane UI is consuming child-session edges
- no concrete reconnect flow depends on cross-node session attach yet

Without a consumer, more session-durability depth would be prerequisite work
stacked on prerequisite work.

## Why programmable topology is next

Programmable topology targets the part of Fireline that is still obviously too
hard-coded: ACP component composition.

Today the runtime builds its ACP chain directly in
[`src/routes/acp.rs`](/Users/gnijor/gurdasnijor/fireline/src/routes/acp.rs).
That is acceptable for the initial substrate slices, but it is the wrong
steady-state seam for the concerns users actually want to add:

- audit
- context injection
- approval gates
- budget caps
- recording / replay
- richer peer routing

Those are all natural `ConnectTo`-style components. They also have concrete
consumer pull that session-graph navigation does not.

## Decision

1. Keep slice 10 deferred until there is a real shared-session consumer.
2. Keep remote child-session attach deferred until a concrete control-plane or
   product consumer appears.
3. Treat programmable topology as the next shipping track.
4. Start with a narrow first mover that proves registry-driven component
   composition without reopening runtime/session semantics.

## Immediate follow-on

The next committed execution doc is
[`12-programmable-topology-first-mover.md`](./12-programmable-topology-first-mover.md).

That slice should:

- introduce `client.topology` as a documented primitive surface
- move optional ACP component composition behind a `ComponentRegistry`
- keep `LoadCoordinatorComponent` as a fixed system component
- prove three components:
  - `peer_mcp`
  - `audit`
  - `context_injection`
