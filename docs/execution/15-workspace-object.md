# 15: Workspace Object

Status: planned
Type: execution slice

Related:

- [`../product/workspaces.md`](../product/workspaces.md)
- [`../product/object-model.md`](../product/object-model.md)
- [`../product/product-api-surfaces.md`](../product/product-api-surfaces.md)
- [`../product/priorities.md`](../product/priorities.md)
- [`../product/roadmap-alignment.md`](../product/roadmap-alignment.md)
- [`./14-runs-and-sessions-api.md`](./14-runs-and-sessions-api.md)
- [`./13-distributed-runtime-fabric/13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md`](./13-distributed-runtime-fabric/13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md)

## Objective

Prove the first real `Workspace` product object so Fireline can answer:

- "run on this folder"
- "use this repo"
- "snapshot this and run elsewhere later"

without treating those answers as ad hoc runtime bootstrap flags.

This first cut should stay intentionally narrow:

- local path workspace
- git workspace
- explicit snapshot operation

It should establish stable workspace identity before remote-sync and provider
details get more complex.

## Product Pillar

Portable workspaces.

## User Workflow Unlocked

Users and host products can:

- connect a local folder once and reuse it across runs
- register a git repository as a reusable source object
- create an explicit snapshot before later or remote execution
- inspect which source context a prior run actually used

## Why This Slice Exists

Without a workspace object, Fireline will keep leaking "what the agent works
on" through:

- local filesystem paths
- runtime-provider mount details
- UI-only remembered context
- one-off bootstrap flags

That becomes a real product problem as soon as Fireline tries to support:

- multiple runs over the same project
- local-to-remote movement
- durable reopen and inspection
- device-independent session history

## Scope

### 1. Workspace object and identity

Define a first-cut `Workspace` product object with stable identity separate
from runtime, session, and capability profile.

Required first-cut fields:

- `workspaceId`
- `name?`
- `source`
- `mode?`
- `createdAtMs`
- `updatedAtMs`

Required first-cut source kinds:

- `{ kind: "local_path"; path: string }`
- `{ kind: "git"; repoUrl: string; ref?: string }`
- `{ kind: "snapshot"; snapshotId: string }`

### 2. Product API surface

Add first-cut product-layer workspace APIs:

```ts
client.workspaces.connectLocal({ path, name? })
client.workspaces.connectGit({ repoUrl, ref?, name? })
client.workspaces.get(workspaceId)
client.workspaces.list(filter?)
client.workspaces.snapshot(workspaceId)
```

The product API should lead with connection and reuse, not low-level mount
configuration.

### 3. Run and session linkage

Runs should consume workspaces explicitly rather than carrying raw path or repo
inputs directly.

This slice should make explicit:

- how a run links to `workspaceId`
- how a session records which workspace it ran against
- how a prior run/session shows the workspace source context it used

### 4. Snapshot as an explicit operation

Snapshotting should be a deliberate product action, not an implicit provider
detail.

This slice should define:

- what object `snapshot(...)` returns
- whether the snapshot becomes a new workspace or an immutable workspace mode
- how later runs can reference that frozen source

The exact transfer protocol is out of scope here; the product object is not.

### 5. First-cut reuse semantics

This slice should prove:

- one workspace can back multiple runs
- one workspace identity can survive across repeated use
- users can distinguish live local context from frozen snapshot context

## Explicit Non-Goals

This slice does **not** require:

- a full sync protocol
- rsync vs archive vs provider-native transfer decisions
- provider-specific mount semantics
- automatic remote materialization
- team-shared workspace infrastructure
- IDE/editor integration details

## Acceptance Criteria

- `Workspace` exists as an explicit product object with stable identity
- local path and git are both first-class workspace source kinds
- `client.workspaces.connectLocal/connectGit/get/list/snapshot` exist
- runs can reference `workspaceId` rather than repeating raw path or repo input
- sessions retain enough workspace linkage to answer "what source context was
  this run using?"
- snapshot creation has a first-cut product contract and yields a reusable
  frozen source reference

## Validation

- `cargo test -q`
- `pnpm --filter @fireline/client test`
- one TypeScript integration test that:
  - connects a local-path workspace
  - starts multiple runs against the same workspace
  - snapshots the workspace
  - starts a run against the snapshot-backed source
- one product-surface integration test that:
  - inspects a prior run/session
  - shows which workspace source it used without relying on raw bootstrap
    flags

## Handoff Note

Keep this slice about product identity, not sync internals.

Do not:

- design a full remote-file transport here
- leak provider mount details into the main product API
- collapse workspace into runtime placement
- collapse workspace into session history

The key proof is simple:

- a workspace is a reusable working-context object
- runs consume it
- sessions remember it

