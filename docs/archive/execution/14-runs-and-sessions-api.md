# 14: Session Canonical Read Surface + Durable Runtime Spec

Status: planned
Type: execution slice

Related:

- [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md)
- [`../product/runs-and-sessions.md`](../product/runs-and-sessions.md)
- [`../product/priorities.md`](../product/priorities.md)
- [`../state/session-load.md`](../state/session-load.md)
- [`../runtime/control-and-data-plane.md`](../runtime/control-and-data-plane.md)
- [`./13-distributed-runtime-fabric/13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md`](./13-distributed-runtime-fabric/13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md)
- [`./17-out-of-band-approvals.md`](./17-out-of-band-approvals.md)

## Primitive Anchor

Primitive extended: `Session`

Acceptance-bar items this slice closes:

- canonical row schema documented and stable
- TypeScript materialization layer with replay/catch-up semantics that
  downstream products can embed
- explicit distinction between hot ACP traffic and cold read-oriented state in
  the TS surface
- `runtimeSpec` durably persisted as a Session event at provision time so a
  later `resume(sessionId)` helper can cold-start the runtime from Session
  evidence alone

Depends on:

- slice `13a` for control-plane-backed runtime descriptors and durable endpoint
  objects
- the existing durable-streams + runtime materializer substrate already shipped

Unblocks:

- the TS-side `resume(sessionId)` helper described in
  [`managed-agents-mapping.md`](../explorations/managed-agents-mapping.md)
- the first worked Orchestration-composition consumer in
  [`17-out-of-band-approvals.md`](./17-out-of-band-approvals.md)
- downstream UIs and control planes that need durable session inspection
  without reverse-engineering raw rows

## Objective

Stabilize `Session` as Fireline's canonical durable read surface and persist the
runtime's provision-time spec as Session evidence.

This slice is not "Run as a product object" and it is not "Session CRUD." It is
the read contract downstream products embed when they need to answer:

- what durable session lineage exists for this work
- what happened in it
- what child sessions, artifacts, permissions, and prompts belong to it
- what metadata is required to reopen or resume it later
- what exact `runtimeSpec` was used to provision the runtime in the first place

The first cut should stay intentionally narrow:

- document a canonical row schema over the durable stream
- ship replay/catch-up materialization helpers in TypeScript
- define read semantics for timeline, children, artifacts, and reopen metadata
- persist `runtimeSpec` as Session evidence at provision time
- make the hot/cold split explicit: ACP is the hot transport, Session is the
  cold durable read surface

## Product Pillar

Durable session read surface.

## User Workflow Unlocked

A control-plane UI, browser surface, or downstream product can consume Fireline
state directly instead of inferring durable history from runtime descriptors,
transport handles, and ad hoc query logic.

The workflow unlocked by this slice is:

- read durable session state through stable collections and helpers
- replay from any offset and catch up to live state
- inspect timeline, artifacts, and child-session lineage
- recover the exact provision-time `runtimeSpec` for a session
- hand that evidence to a later `resume(sessionId)` helper without relying on a
  second catalog

The consumer here is not a new `client.runs.*` namespace. It is any product
that needs a trustworthy durable read contract over Fireline's existing session
substrate.

## Scope

### 1. Canonical session read schema

Define the durable row families Fireline treats as canonical Session evidence.

At minimum this slice should name and stabilize the rows or projections that
downstream consumers may rely on:

- `runtime`
- `session`
- `prompt_turn`
- `permission`
- `terminal`
- `chunk`
- child-session lineage edges
- first-cut artifact evidence rows or artifact projections
- a provision-time `runtimeSpec` event or projection keyed back to the session
  and runtime

For each row family, the schema surface should answer:

- what durable fact this row represents
- what its primary key or identity is
- how it links back to `runtime_key` and `session_id`
- whether it is append-only evidence, latest-state projection, or derived
  collection state

This slice should prefer honest row-level vocabulary over aspirational product
objects. If a downstream product wants a `Run` view, it can derive it from this
schema plus orchestration state.

### 2. Session read model

Define the first read model that consuming products can treat as stable.

Required first-cut read surface:

- `sessionId`
- `parentSessionId?`
- `rootSessionId`
- `runtimeKey?`
- `runtimeId?`
- `nodeId?`
- `runtimeSpec?`
- started/updated/ended timestamps derived from durable evidence
- resumability or reopen metadata derived from durable state
- lineage references for children and artifacts

The important constraint is that this is a durable read model over existing
rows and events. It is not a new hidden event model and it is not a new source
of truth.

### 3. Read surface shape

Expose TypeScript helpers that sit above raw collections without pretending to
be a full Fireline-owned product API.

The helpers should make these reads straightforward:

- get session header/detail by `sessionId`
- stream or materialize timeline entries for a session
- list child sessions for a parent session
- list first-cut artifacts for a session
- follow runtime/session lineage needed for reopen or resume
- read the provision-time `runtimeSpec` associated with the session
- replay from a known durable offset and catch up to live state

This is intentionally narrower than:

```ts
client.runs.start()
client.runs.list()
client.sessions.resume()
```

Those are downstream product surfaces. Fireline's job here is to make the
durable read substrate clean enough that those surfaces can be built without
guesswork.

### 4. Durable `runtimeSpec` persistence

Persist the runtime's provision-time spec as Session evidence when the runtime
is created.

This slice should make explicit:

- when the `runtimeSpec` event is written
- which fields of the launch spec are preserved
- how the event links to `runtime_key` and the root session
- how consumers read the stored spec back through the Session surface

The key constraint is architectural:

- `resume(sessionId)` should be able to recover the cold-start input from
  Session evidence
- no second hidden runtime catalog should be required for the basic resume path

If a future control plane also keeps a catalog copy, that is additive. The
Session surface still needs the durable spec because the reduction in
`managed-agents-mapping.md` depends on Session evidence being sufficient.

### 5. Durable evidence mapping

This slice must anchor every read helper in durable Session evidence from the
managed-agents mapping doc.

The main rules are:

- no second hidden event model
- no UI-only synthetic timeline state
- no reopen semantics based on ephemeral runtime memory
- no artifact listing that bypasses durable linkage
- no resume path that depends on a launch spec hidden outside Session evidence

If a read cannot be explained as "derived from the durable stream and its
materialized collections," it does not belong in this slice.

### 6. Resume and reopen semantics

Define what metadata a consumer product needs from the Session read surface in
order to reopen or resume work through the composition pattern described in
`managed-agents-mapping.md` §2.

This slice should make explicit:

- how a consumer knows whether a session is reopenable or resumable
- what durable lineage links a later runtime back to the prior session
- how `runtimeSpec` is recovered for cold-start resume
- which session metadata is sufficient for `session/load`-style reopen flows
- which concerns belong to Session reads versus later composition glue such as
  `resume(sessionId)`

The key boundary is:

- `Session` tells consumers what durable work exists and how it links together
- `resume(sessionId)` composes Session reads with provision + ACP reconnect

### 7. First consumer path

One real consumer should prove that the read surface is sufficient without
reaching through to raw row interpretation in application code.

Good candidates:

- browser harness or browser contract tests
- a control-plane UI view
- a thin diagnostic page or CLI read path

The proof should show a consumer reading:

- session detail
- timeline
- children
- artifacts or first-cut artifact evidence
- `runtimeSpec`
- reopen metadata

without inventing its own schema on the side.

## Explicit Non-Goals

This slice does **not** require:

- `Run` as a Fireline-owned live product object
- `client.runs.start/get/list`
- `client.sessions.resume(...)` as a first-class Fireline product API
- a REST CRUD surface for sessions
- a new event store
- replacing ACP as the live write path
- a separate orchestration namespace or wake endpoint
- a complete artifact product model beyond the first durable read answer

Run belongs to orchestration and downstream product projection, not to this
read-surface slice.

## Acceptance Criteria

- the canonical durable row schema for Session reads is documented and treated
  as stable
- the TypeScript read surface supports replay from a known offset and catch-up
  to live state
- the TS surface makes the hot/cold split explicit:
  - ACP is the hot transport for live prompts and completions
  - Session reads are the cold durable surface for inspection and restore
- a provision-time `runtimeSpec` event is part of the canonical Session surface
- consumers can read, from the canonical surface:
  - session detail
  - timeline
  - child-session lineage
  - first-cut artifact evidence
  - `runtimeSpec`
  - reopen/resume metadata
- at least one consumer proves those reads without hand-decoding raw stream
  rows inside product code
- the read model is sufficient input for a later `resume(sessionId)` helper,
  but it does not itself own run lifecycle or orchestration

## Validation

- `cargo test -q`
- `pnpm --filter @fireline/client test`
- one replay/catch-up integration test that:
  - materializes session state from a known offset
  - catches up to new durable events
  - verifies the consumer-facing collections stay consistent
- one `runtimeSpec` persistence test that:
  - provisions a runtime
  - verifies the spec is written as Session evidence
  - reconstructs the spec from the read surface
  - proves that recovered spec is sufficient input for a later cold-start
    resume path
- one consumer-oriented TypeScript or UI integration test that:
  - reads session detail, timeline, children, artifacts, and `runtimeSpec`
  - disconnects and reconnects
  - proves reopen metadata can be recovered from durable state without touching
    ACP first

## Handoff Note

Keep this slice narrow and substrate-first.

The handoff should emphasize:

- do not build `client.runs.*`
- do not build session CRUD
- do build the canonical schema and TS replay/read helpers
- do persist `runtimeSpec` as Session evidence at provision time
- do make the hot ACP path and cold durable read path explicit

Session is Fireline's durable read surface. `runtimeSpec` is part of that
surface. `resume(sessionId)` is downstream composition over it.
