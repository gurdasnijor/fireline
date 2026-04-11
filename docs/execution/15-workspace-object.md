# 15: Resources Primitive Refactor

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

- `resources: Vec<ResourceRef>` field on `CreateRuntimeSpec`
- `ResourceMounter` trait
- documented contract for how mounters interact with `RuntimeProvider::start()`
- the first end-to-end proof that a runtime launches with mounted resources

This is intentionally smaller than a normal numbered slice. It is roughly a
one-week substrate refactor, not a new Fireline product object.

Depends on:

- the existing `RuntimeProvider` launch path
- slice `13c` if the first non-local provider ends up contributing the initial
  `LocalPath` mounting behavior

Unblocks:

- later non-local resource mounters under the same contract
- downstream products that want to point Fireline at local paths, git refs, or
  object-store refs without inventing a workspace database

## Objective

Replace "Workspace as a product object" with the smaller Resources primitive:

- `resources: [{ source_ref, mount_path }]`
- pluggable mounters that materialize those refs before runtime launch

The first cut should stay intentionally narrow:

- add `resources` to launch specs
- define `ResourceRef`
- define `ResourceMounter`
- name the first four mounter implementations:
  - `LocalPath`
  - `GitRemote`
  - `S3`
  - `GCS`

This refactor is about portable execution inputs, not product identity.

## Product Pillar

Resources.

## User Workflow Unlocked

A consuming product can point Fireline at:

- a local path
- a git repository or ref
- an object-store-backed source

without inventing a heavyweight workspace system first.

The unlocked workflow is simple:

- package resource references into the runtime launch spec
- let Fireline materialize those resources into known mount paths
- start the runtime against the mounted inputs

## Why This Slice Exists

Today source context leaks through ad hoc launch details:

- local filesystem paths
- provider-specific mount flags
- helper-file routes that assume the host filesystem is the source of truth
- one-off bootstrap fields

The managed-agents anchor collapses this to a much smaller substrate shape:

- a resource reference
- a mount path
- a mounter that knows how to fetch or bind the source

That means the current "workspace object" framing is too large for the problem
Fireline actually needs to solve at the substrate layer.

## Scope

### 1. `ResourceRef` shape

Define the launch-spec shape Fireline accepts for portable execution inputs.

First-cut shape:

```ts
type ResourceRef = {
  source_ref: string;
  mount_path: string;
  mode?: "read_only" | "read_write";
  fetch_mode?: "bind" | "clone" | "download";
};
```

The important fields are:

- `source_ref`
  A provider-neutral reference to the source material.
- `mount_path`
  Where the runtime should see the materialized resource.

Optional fields can remain narrow in the first cut. The important thing is that
resource identity lives in the launch spec, not in a separate Fireline-owned
workspace object.

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

This is the entire substrate job. Catalog UIs, workspace identity, and snapshot
management belong elsewhere.

### 3. Initial implementations

This refactor should explicitly define four implementations under the same
contract:

- `LocalPathMounter`
- `GitRemoteMounter`
- `S3Mounter`
- `GcsMounter`

The first landing does not need all four to have equal depth, but it should
name them as the initial contract surface so later additions do not invent
different abstractions for each source kind.

Recommended first landing:

- `LocalPathMounter` as the direct replacement for today's implicit local-path
  behavior
- `GitRemoteMounter` as the first network-fetched proof

`S3Mounter` and `GcsMounter` can remain thinly specified if needed, but they
should be named here so the trait surface is designed for object-store sources,
not only filesystem paths.

### 4. Launch-spec integration

Wire `resources` into `CreateRuntimeSpec` or the equivalent launch input so
every provider sees the same resource contract.

This slice should make explicit:

- where `resources` lives in the runtime create path
- how the chosen mounter outputs feed into `RuntimeProvider::start()`
- how mounted resources are reflected, if at all, in runtime descriptors or
  session evidence
- what "no matching mounter" and "materialization failed" errors look like

This replaces the old "workspace linkage" language. Runs and sessions may later
record which resources were used, but Resources itself is a launch input
primitive.

### 5. Initial materialization behavior

Document the first-cut behavior for each source kind.

At minimum:

- `LocalPathMounter` bind-mounts or directly exposes an existing local path
- `GitRemoteMounter` clones or fetches a repo/ref into a materialized path
- `S3Mounter` downloads an object or prefix into a materialized path
- `GcsMounter` does the same for Google Cloud Storage

The exact transport or caching strategy can stay narrow in the first cut. The
important contract is that all four produce materialized resources at
`mount_path` before the runtime starts.

## Explicit Non-Goals

This refactor does **not** require:

- a Fireline-owned `Workspace` product object
- `client.workspaces.*`
- a workspace database or catalog
- snapshot identity as a first-class Fireline substrate concern
- remote sync architecture beyond what each mounter needs
- provider-specific resource APIs leaking into the main launch spec

If a consuming product wants reusable named workspaces, it can build them on
top of `ResourceRef` lists.

## Acceptance Criteria

- runtime launch specs accept `resources: ResourceRef[]`
- `ResourceRef` is documented with `source_ref` and `mount_path` as the core
  stable fields
- `ResourceMounter` exists as the shared materialization seam
- the contract explicitly names:
  - `LocalPathMounter`
  - `GitRemoteMounter`
  - `S3Mounter`
  - `GcsMounter`
- at least one provider path proves:
  - a local resource can be mounted through the new contract
  - a non-local resource kind can be materialized through the same contract
- the launch path documents how mounter output feeds into
  `RuntimeProvider::start()`

## Validation

- `cargo test -q`
- one provider/bootstrap integration test for `LocalPathMounter`
- one provider/bootstrap integration test for a network-fetched resource kind
  such as `GitRemoteMounter`
- one runtime smoke test that proves the agent can read mounted material from
  the requested `mount_path`

## Handoff Note

Keep this doc and the implementation small.

The handoff should emphasize:

- this is a Resources primitive refactor, not a workspace product slice
- the deliverable is `ResourceRef` + `ResourceMounter` + launch-spec wiring
- `LocalPath`, `GitRemote`, `S3`, and `GCS` should all fit the same contract
- do not invent `client.workspaces.*`
- do not create a Fireline-managed workspace identity system here

This is a small refactor, not a new Fireline product object.
