# 14: Runs and Sessions API

Status: planned
Type: execution slice

Related:

- [`../product/runs-and-sessions.md`](../product/runs-and-sessions.md)
- [`../product/object-model.md`](../product/object-model.md)
- [`../product/product-api-surfaces.md`](../product/product-api-surfaces.md)
- [`../product/priorities.md`](../product/priorities.md)
- [`../product/roadmap-alignment.md`](../product/roadmap-alignment.md)
- [`../state/session-load.md`](../state/session-load.md)
- [`./13-distributed-runtime-fabric/13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md`](./13-distributed-runtime-fabric/13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md)

## Objective

Prove the first real product-layer surface above Fireline's existing systems
primitives:

- `Run` as the live managed execution object
- `Session` as the durable evidence object

This slice should turn those into explicit product APIs instead of leaving them
as implicit combinations of runtime descriptors, raw state rows, and ACP
transport handles.

The scope should stay intentionally narrow:

- build on top of the existing runtime/session substrate
- project honest product objects
- do not reopen provider, auth, or bootstrap work from slice `13`

## Product Pillar

Durable sessions.

## User Workflow Unlocked

From a browser, CLI, or control-plane UI:

- list the runs that currently matter
- inspect what happened in a session
- reopen or resume prior work after the original client disconnects
- inspect transcript, artifacts, and child-session lineage without reasoning
  about raw ACP endpoints or runtime registry details

## Why This Slice Exists

Fireline already has strong session substrate:

- durable `session`, `prompt_turn`, `chunk`, and lineage rows
- runtime-owned session lifetime
- local `session/load` coordination
- TypeScript-side durable state materialization

What it does not yet have is a product surface that answers:

- what run is live right now?
- what session is the durable record of that run?
- what can I reopen later?
- what child sessions or artifacts belong to that work?

Without this slice, Fireline still looks too much like a runtime toolkit and
not enough like a durable agent product.

## Scope

### 1. Run object projection

Define a first-cut `Run` product object that is separate from raw runtime
metadata and separate from raw session rows.

Required first-cut fields:

- `runId`
- `rootSessionId`
- `workspaceId?`
- `profileId?`
- `placement?`
- `state`
- `blockingRequestIds?`
- `createdAtMs`
- `updatedAtMs`

Required first-cut run states:

- `starting`
- `running`
- `waiting`
- `completed`
- `failed`
- `cancelled`

This slice does not need to solve every lifecycle transition, but it must make
the run object explicit and queryable.

### 2. Session object projection

Define a first-cut `Session` product object that sits above the current durable
state rows.

Required first-cut fields:

- `sessionId`
- `runId?`
- `parentSessionId?`
- `runtimeId?`
- `runtimeKey?`
- `nodeId?`
- `state`
- `resumable`
- `startedAtMs`
- `endedAtMs?`

The session object should answer durable-history questions, not runtime-control
questions.

### 3. Product API surface

Add first-cut product-layer namespaces:

```ts
client.runs.start(spec)
client.runs.get(runId)
client.runs.list(filter?)

client.sessions.get(sessionId)
client.sessions.list(filter?)
client.sessions.resume(sessionId, options?)
client.sessions.timeline(sessionId)
client.sessions.artifacts(sessionId)
client.sessions.children(sessionId)
```

The implementation may compile down into existing substrate primitives such as:

- `client.host`
- `client.acp`
- `client.state`

but the product layer must stop exposing those primitives as the only way to
reason about durable work.

### 4. Durable evidence mapping

This slice must explicitly map product objects to durable evidence already
present in the system.

At minimum:

- `Run.rootSessionId` must be explicit
- session timeline must be backed by durable prompt/chunk/message evidence
- session children must be backed by durable child-session lineage
- artifact inspection must have a first-cut answer, even if the artifact story
  is initially narrow

The product layer should not invent a second hidden event model.

### 5. Resume and reopen semantics

`client.sessions.resume(...)` must be defined as a product action above the
existing `session/load` substrate.

This slice should make explicit:

- when a session is resumable
- what product object is resumed
- how a resumed run links back to the prior durable session lineage

### 6. First consumer path

One real consumer path should be able to use these product objects directly.

Examples:

- browser harness
- control-plane UI
- a future CLI command set

The point is to prove that product consumers can stop reaching directly for raw
runtime descriptors and raw state collections.

## Explicit Non-Goals

This slice does **not** require:

- `client.runs.move(...)`
- `client.runs.retry(...)`
- full stop/cancel semantics beyond what already exists below the product layer
- a final REST query language for runs and sessions
- a new event store or schema rewrite
- solving shared-session or multi-viewer semantics
- broad replay-engine work

## Acceptance Criteria

- `Run` and `Session` exist as explicit product objects rather than being
  implied by raw runtime/state surfaces
- `client.runs.start/get/list` exist with explicit `rootSessionId` linkage
- `client.sessions.get/list/resume/timeline/artifacts/children` exist
- a session can be resumed after the original client disconnects through the
  product-layer surface
- run state can represent `waiting` even if approval service lands in a later
  slice
- at least one product consumer can answer:
  - "what needs attention right now?" from `runs`
  - "what happened and what can I reopen?" from `sessions`

## Validation

- `cargo test -q`
- `pnpm --filter @fireline/client test`
- one TypeScript integration test that:
  - starts work through `client.runs.start(...)`
  - lists runs and sessions
  - fetches session timeline and children
  - disconnects and resumes the same session through `client.sessions.resume(...)`
- one consumer-oriented integration test that:
  - renders or queries run list and session detail through the product API
  - does not rely on direct inspection of raw runtime descriptors or raw
    StreamDB collections

## Handoff Note

Build this slice as a projection layer on top of existing honest primitives.

Do not:

- reopen runtime-fabric/auth work from slice `13`
- invent a second execution model
- make `Run` just a renamed `RuntimeDescriptor`
- make `Session` just a direct passthrough of the underlying state row

The key proof is product clarity:

- `run` is the live control object
- `session` is the durable evidence object

