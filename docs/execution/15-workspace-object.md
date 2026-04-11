# 15: Resources — Physical Mounts + FsBackendComponent

Status: planned
Type: execution refactor

Related:

- [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md)
- [`../product/workspaces.md`](../product/workspaces.md)
- [`../product/priorities.md`](../product/priorities.md)
- [`../runtime/control-and-data-plane.md`](../runtime/control-and-data-plane.md)
- [`./13-distributed-runtime-fabric/13c-first-remote-provider-docker-via-bollard.md`](./13-distributed-runtime-fabric/13c-first-remote-provider-docker-via-bollard.md)
- [`./14-runs-and-sessions-api.md`](./14-runs-and-sessions-api.md)

## Primitive Anchor

Primitive extended: `Resources`

Acceptance-bar items this refactor closes:

Physical mounts:

- `resources: Vec<ResourceRef>` field on `CreateRuntimeSpec`
- `ResourceMounter` trait on the runtime provider side
- documented contract for how mounters interact with `RuntimeProvider::start()`
- one end-to-end proof that a runtime mounts a non-local resource and the agent
  reads it from the filesystem

ACP fs interception:

- `FileBackend` trait on the runtime side
- `FsBackendComponent` in `fireline-components` implemented as
  `compose(substitute, appendToSession)`
- `LocalFileBackend` and `SessionLogFileBackend` as the first two
  implementations
- one end-to-end proof that ACP-native fs writes land in the Session log and
  surface as artifact evidence

This is still smaller than a traditional execution slice. It is one focused
Resources refactor with two complementary halves.

Depends on:

- the existing `RuntimeProvider` launch path
- the conductor topology/component seam from slice `12`
- slice `13c` if the first non-local provider contributes initial
  `LocalPathMounter` behavior as a side effect

Unblocks:

- later S3/GCS/git backends under the same `FileBackend` contract
- downstream products that want to point Fireline at local paths, git refs, or
  object-store refs without inventing a workspace database
- ACP-native artifact capture and virtual-fs patterns on top of the Session log

## Objective

Replace the old "Workspace object" framing with the actual Resources work
Fireline needs to ship:

- physical mounts for shell-based agents
- ACP fs interception via a composable `FsBackendComponent`

The first cut should stay intentionally narrow:

- add `CreateRuntimeSpec.resources`
- define `ResourceRef`
- define `ResourceMounter`
- ship `LocalPathMounter` and `GitRemoteMounter`
- define `FileBackend`
- ship `FsBackendComponent` in `fireline-components`
- ship `LocalFileBackend` and `SessionLogFileBackend`

This refactor is about portable execution inputs and ACP-native file routing,
not product identity.

## Product Pillar

Resources.

## User Workflow Unlocked

A consuming product can do two things under one coherent Resources contract:

- point shell-based agents at real files on disk by mounting local or fetched
  sources before runtime launch
- point ACP-native file operations at a chosen backend without inventing a new
  primitive

That unlocks:

- local path and git-backed working directories for shell-based agents
- ACP-native file reads and writes routed to local disk or the Session log
- artifact capture that falls out of Session appends instead of a separate
  artifact subsystem

## Why This Slice Exists

Resources turned out to split into two distinct halves.

The first half is **not composable away**:

- Claude Code, Codex, and other shell-based agents read and write via
  bash/python/their own tools
- those file operations happen inside the runtime filesystem
- we cannot reliably intercept arbitrary shell I/O through ACP

So physical files still need to exist at `mount_path` before the runtime
starts. That is the `ResourceMounter` half.

The second half **is composable**:

- ACP defines `fs/read_text_file` and `fs/write_text_file`
- those requests flow through the conductor proxy chain
- a component can substitute the backend and append writes to Session

That is the `FsBackendComponent` half.

Both layers are needed. One runtime can use both at once.

## Scope

### 1. `ResourceRef` and physical mounts

Define the launch-spec shape Fireline accepts for physical execution inputs.

First-cut shape:

```ts
type ResourceRef = {
  source_ref: string
  mount_path: string
  mode?: "read_only" | "read_write"
  fetch_mode?: "bind" | "clone" | "download"
}
```

The important fields are:

- `source_ref`
  Provider-neutral reference to the source material.
- `mount_path`
  Where the runtime should see the materialized resource.

This slice should wire `resources: ResourceRef[]` into `CreateRuntimeSpec` or
the equivalent launch input so every provider sees the same resource contract.

### 2. `ResourceMounter` trait

Define the substrate seam that materializes a `ResourceRef` before runtime
launch.

Responsibilities:

- inspect a `ResourceRef`
- decide whether the mounter can handle it
- materialize the resource into or at `mount_path`
- return the launch-time mount information the provider needs

The contract should make the ordering explicit:

1. launch spec arrives with `resources`
2. `RuntimeProvider::start()` resolves the matching mounters
3. resources are materialized
4. the runtime launches against the resulting mounts

The first two implementations should be:

- `LocalPathMounter`
- `GitRemoteMounter`

`S3Mounter` and `GcsMounter` remain follow-ups under the same trait.

### 3. `FileBackend` trait

Define the runtime-side trait for ACP-native file operations.

Responsibilities:

- read a file by logical path
- write a file by logical path and content
- optionally expose metadata or existence checks later without changing the
  basic routing contract

The first two implementations should be:

- `LocalFileBackend`
- `SessionLogFileBackend`

`LocalFileBackend` mirrors today's local-disk behavior for ACP fs methods.
`SessionLogFileBackend` stores file content as Session events and reads back via
projection, making the Session log itself a virtual filesystem for small,
durable workflows.

### 4. `FsBackendComponent`

Add a built-in `FsBackendComponent` in `fireline-components`.

The shape should follow the managed-agents mapping directly:

- `compose(substitute, appendToSession)`

Responsibilities:

- intercept ACP `fs/read_text_file` and `fs/write_text_file`
- route them to the configured `FileBackend`
- append a durable `fs_op` event to Session so writes become artifact evidence

The important constraint is that this is a component, not a new primitive. It
belongs in the conductor topology alongside `audit`, `approvalGate`, and other
components.

### 5. How the two halves coexist

This refactor should explicitly document that physical mounts and ACP fs
interception complement each other rather than compete:

- shell-based reads and writes use the mounted filesystem
- ACP-native fs operations use `FsBackendComponent`
- ACP-native writes are durably logged through `appendToSession`

A single runtime can therefore:

- have `/work` populated by `ResourceMounter`
- serve ACP file operations through `LocalFileBackend` or
  `SessionLogFileBackend`
- use the Session log as artifact evidence for ACP-native writes

### 6. First implementation boundary

What slice 15 actually ships:

1. physical mounts:
   - `ResourceRef`
   - `CreateRuntimeSpec.resources`
   - `ResourceMounter`
   - `LocalPathMounter`
   - `GitRemoteMounter`
2. ACP fs interception:
   - `FileBackend`
   - `FsBackendComponent`
   - `LocalFileBackend`
   - `SessionLogFileBackend`

This is enough to prove both halves of Resources without overcommitting to S3,
GCS, or a full workspace product.

## Explicit Non-Goals

This refactor does **not** require:

- a Fireline-owned `Workspace` product object
- `client.workspaces.*`
- a workspace database or catalog
- snapshot identity as a first-class Fireline substrate concern
- shell-level interception of arbitrary file operations
- `S3Mounter` or `GcsMounter` in the first landing
- `S3FileBackend`, `GcsFileBackend`, or `GitFileBackend` in the first landing

If a consuming product wants reusable named workspaces, it can build them on
top of `ResourceRef` lists.

## Acceptance Criteria

Physical mounts:

- runtime launch specs accept `resources: ResourceRef[]`
- `ResourceRef` is documented with `source_ref` and `mount_path` as the stable
  core fields
- `ResourceMounter` exists as the shared materialization seam
- `LocalPathMounter` and `GitRemoteMounter` are defined as the first concrete
  implementations
- the launch path documents how mounter output feeds into
  `RuntimeProvider::start()`
- one provider path proves a non-local resource can be mounted and read via the
  runtime filesystem

ACP fs interception:

- `FileBackend` exists as the runtime-side file routing seam
- `FsBackendComponent` exists in `fireline-components` and is explicitly
  modeled as `compose(substitute, appendToSession)`
- `LocalFileBackend` and `SessionLogFileBackend` exist as the first two
  implementations
- one end-to-end proof shows:
  - an agent performs `fs/write_text_file`
  - the configured backend handles the write
  - an `fs_op` event lands on the Session log
  - a materializer can surface that write as artifact evidence

## Validation

- `cargo test -q`
- one provider/bootstrap integration test for `LocalPathMounter`
- one provider/bootstrap integration test for `GitRemoteMounter`
- one runtime smoke test that proves a shell-based agent can read mounted
  material from the requested `mount_path`
- one conductor/component integration test that proves ACP-native
  `fs/write_text_file` goes through `FsBackendComponent` and lands on the
  Session log

## Handoff Note

Keep this refactor small and two-layered.

The handoff should emphasize:

- this is a Resources primitive refactor, not a workspace product slice
- the physical-mount half is for shell-based agents
- the `FsBackendComponent` half is for ACP-native fs operations
- `FsBackendComponent` is composition, not a new primitive
- do not invent `client.workspaces.*`
- do not create a Fireline-managed workspace identity system here

The success condition is not "Fireline has workspaces." It is:

- shell-based agents can start with the right files mounted
- ACP-native file operations can route through pluggable backends
- ACP-native writes become durable Session evidence automatically
