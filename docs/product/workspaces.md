# Workspaces

> Related:
> - [`index.md`](./index.md)
> - [`object-model.md`](./object-model.md)
> - [`product-api-surfaces.md`](./product-api-surfaces.md)
> - [`user-surfaces.md`](./user-surfaces.md)
> - [`priorities.md`](./priorities.md)
> - [`roadmap-alignment.md`](./roadmap-alignment.md)
> - [`../runtime/lightweight-runtime-provider.md`](../runtime/lightweight-runtime-provider.md)
> - [`../execution/13-distributed-runtime-fabric/README.md`](../execution/13-distributed-runtime-fabric/README.md)

## Purpose

Fireline needs a clear product answer to:

- "run on this folder"
- "use this repo"
- "move this work to a remote runtime"
- "come back later and keep working against the same project"

That answer should not be "just pass a local path into runtime bootstrap."

It should be a first-class **workspace** object.

## What A Workspace Is

A workspace is the portable working context for a run.

It defines:

- what files or source material the run should see
- how that material is identified
- how it should be mounted, synced, or materialized for execution

Examples:

- a local folder on the current machine
- a git repository at a particular ref
- a previously captured snapshot
- a provider-specific mounted project root

## What A Workspace Is Not

A workspace is not:

- a runtime
- a capability profile
- a session transcript
- an artifact bundle produced by a run

Those are separate product objects.

The workspace is the **working set**, not the execution substrate and not the
result history.

## Why It Needs To Exist

Without a workspace model, Fireline risks treating "what the agent works on" as
an ad hoc mixture of:

- local filesystem flags
- runtime provider mounts
- UI-only remembered paths
- one-off upload flows

That would make core user workflows brittle:

- start locally, continue remotely
- resume later from another device
- reuse the same project context across many runs
- understand what a prior run was actually operating against

The workspace object gives Fireline a stable identity for that context.

## Product Questions A Workspace Should Answer

- what kind of workspace is this?
- where did it come from?
- what sync or mount strategy does it use?
- can it be reused on another run?
- can it be moved to a remote runtime?
- is it immutable, live-mounted, or snapshot-based?

## Core Workspace Modes

### 1. Local path workspace

Best for:

- interactive coding on the current machine
- editor-integrated flows
- fast local iteration

The important product behavior is:

- the workspace has a stable identity
- the source path is part of its definition
- runs can reference it without respecifying the raw path every time

### 2. Git workspace

Best for:

- background work
- reproducible runs
- server-side or hosted execution

The important product behavior is:

- the workspace identifies a repo and optional ref
- the runtime materializes that repo as needed
- runs can be recreated or resumed against known source inputs

### 3. Snapshot workspace

Best for:

- portable remote execution
- durable handoff
- "freeze this state and run elsewhere"

The important product behavior is:

- the workspace references a captured snapshot
- the snapshot can be materialized on another runtime later
- the original local machine does not have to stay online

### 4. Mounted or provider-managed workspace

Best for:

- longer-lived remote environments
- team-shared project environments
- provider-specific storage models

This should be allowed, but it should still look like a workspace at the
product layer instead of leaking provider details into the primary UX.

## Workspace Identity Matters More Than Sync Strategy

The most important product decision is:

**workspace identity should be stable even if the sync strategy changes.**

For example, the same logical workspace may:

- start as a live local bind mount
- later become a snapshot for remote execution
- later reconnect to a synced remote copy

That should not require the user to think they are working in a completely
different product object each time.

## Relationship To Runs

Runs consume workspaces.

That means:

- many runs may point at one workspace
- the same workspace may be used across local and remote placement
- run history should tell you which workspace version or source it used

This is critical to making "resume my agent later" coherent.

## Relationship To Sessions

Sessions should record which workspace a run was operating against, but they
should not own the workspace itself.

The workspace must remain reusable after a particular session ends.

## Relationship To Capability Profiles

Workspace answers:

- what files and working material does the run see?

Capability profile answers:

- what tools, credentials, and policies does the run carry?

Those should remain separate so users can mix:

- one workspace with many profiles
- one profile with many workspaces

## Suggested Product Surface

```ts
client.workspaces.connectLocal({ path, name? })
client.workspaces.connectGit({ repoUrl, ref?, name? })
client.workspaces.get(workspaceId)
client.workspaces.list(filter?)
client.workspaces.snapshot(workspaceId)
client.workspaces.materialize(workspaceId, target?)
client.workspaces.archive(workspaceId)
```

The product layer should lead with connection and reuse, not low-level provider
mount configuration.

## Strawman Workspace Shape

```ts
type Workspace = {
  workspaceId: string
  name?: string

  source:
    | { kind: "local_path"; path: string }
    | { kind: "git"; repoUrl: string; ref?: string }
    | { kind: "snapshot"; snapshotId: string }
    | { kind: "mounted"; mountId: string }

  mode?: "live" | "snapshot" | "synced"
  createdAtMs: number
  updatedAtMs: number
}
```

## Local-To-Remote Story

This is where the workspace model becomes especially important.

When a user says:

- "start here on my laptop"
- "now move that to the cloud"

Fireline should be able to answer:

- which workspace identity stays the same
- which sync or snapshot mechanism is used
- whether the remote run sees a live mount, a copy, or a frozen snapshot

That is a product concern before it becomes a provider concern.

## What Should Stay Below The Product Layer

These should remain implementation details unless an advanced operator needs
them:

- rsync vs archive upload vs provider-native sync internals
- raw mount paths inside containers
- bind mount flags
- storage bucket details
- provider-specific workspace bootstrap steps

The product layer should expose the effect, not the transport.

## First-Cut Recommendation

The first product-surface version of workspaces should support only:

1. local path workspaces
2. git workspaces
3. snapshot creation as an explicit operation

That is enough to support the most important user stories without prematurely
locking in a sync implementation.

## What Future Slices Should Prove

Future slices in this area should prove:

- one workspace can back multiple runs
- a workspace can be used across local and remote placement
- a snapshot-backed run can resume without the original laptop remaining online
- users can tell what source context a prior run actually used

## Non-Goals

This doc does not define:

- exact sync protocol
- exact artifact storage format
- full provider-specific mount semantics
- IDE integration details

Those belong to technical docs and execution slices.
