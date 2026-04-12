# Stream-FS spike

> **Status:** narrow spike proposal
> **Type:** design doc
> **Audience:** `fireline-resources`, Host/runtime integration, demo planning
> **Related:**
> - [`../explorations/stream-fs-resources-evaluation.md`](../explorations/stream-fs-resources-evaluation.md)
> - [`./resource-discovery.md`](./resource-discovery.md)
> - [`./cross-host-discovery.md`](./cross-host-discovery.md)
> - [`./deployment-and-remote-handoff.md`](./deployment-and-remote-handoff.md)

This proposal answers one spike question only:

> **Can Fireline demo cross-Host collaborative file access in one commit of
> scaffolding against the existing Durable Streams `packages/stream-fs`
> package?**

The answer is:

- **yes**, if the spike is deliberately narrow
- **no**, if "collaborative filesystem" is interpreted as live writable,
  conflict-aware, cache-coherent multi-writer semantics

The spike should therefore target the narrowest plausible version:

- `StreamFsMounter` as a `ResourceMounter` implementation
- host-side FUSE presentation
- bind-mount into the runtime at `mount_path`
- **pinned-snapshot, read-only** as the first mode

That is enough to prove the ergonomic question:

> can a Host publish a `StreamFs` resource, a second Host discover it, mount it
> as an ordinary path, and let an agent read it without any custom client code?

It is intentionally **not** a commitment to live writable shared workspaces.

## 1. Why this spike exists

The existing evaluation in
[`stream-fs-resources-evaluation.md`](../explorations/stream-fs-resources-evaluation.md)
already made the strategic call:

- do not make `stream-fs` the first implementation of the Resources
  primitive
- if explored at all, explore it as a **later narrow spike**
- start with **read-only pinned snapshot mount**

Since then, two companion proposals sharpened where that spike fits:

- [`resource-discovery.md`](./resource-discovery.md) added `StreamFs` as a
  first-class `ResourceSourceRef` variant
- [`cross-host-discovery.md`](./cross-host-discovery.md) specified the
  durable-streams-backed mechanism by which one Host discovers artifacts
  published by another

That makes the next question much narrower than the original evaluation:

> not "should Fireline adopt stream-fs broadly?"
>
> but "can one small spike prove that a `StreamFs` resource can participate in
> the same publish/discover/mount loop as every other resource kind?"

## 2. Spike goal

The spike goal is a single demo-capable path:

1. Host A publishes a `StreamFs` resource to
   `resources:tenant-<id>`.
2. Host B replays the same resource stream, discovers that resource, and
   resolves a `StreamFs` source ref.
3. Host B mounts the resource through a host-side `stream-fs` FUSE
   mount, pinned to a revision.
4. Host B bind-mounts that local path into the runtime at `mount_path`.
5. An agent running on Host B reads files from that path as if it were an
   ordinary read-only directory.

If that works, the spike is successful.

The spike does **not** need to prove:

- distributed write correctness
- concurrent editing
- rename/move semantics
- watcher/event propagation into the runtime
- cache invalidation under mutation

Those are exactly the parts the evaluation doc identified as risky.

## 3. The proposed `StreamFs` shape

This spike uses the `StreamFs` variant already introduced in
[`resource-discovery.md`](./resource-discovery.md):

```rust
pub enum ResourceSourceRef {
    StreamFs {
        source_ref: String,
        revision: Option<String>,
        mode: StreamFsMode,
    },
    // ...
}

pub enum StreamFsMode {
    SnapshotReadOnly,
    LiveReadOnly,
    LiveReadWrite,
}
```

For the spike, only one mode is in scope:

```rust
StreamFsMode::SnapshotReadOnly
```

Interpretation:

- `source_ref`: the logical stream-fs workspace identifier
- `revision`: the pinned snapshot or revision identity to materialize
- `mode`: explicitly read-only and pinned

### Why `revision` is mandatory for the spike

The evaluation doc is clear that reproducibility is the first hard
question. Fireline's Resources primitive is about portable launch inputs,
not about unbounded live collaboration. Without a stable revision, the
spike would prove only "a Host can mount some changing shared state,"
which is not a useful or repeatable demo.

So the spike must treat `revision` as effectively required even if the
wire field remains typed as `Option<String>` for forward compatibility.

If the existing `stream-fs` package cannot provide a stable revision or
snapshot identity cheaply, the spike should stop there and report that
as the result.

## 4. Proposed architecture

### 4.1 `StreamFsMounter`

The new piece on the Fireline side is:

```rust
pub struct StreamFsMounter {
    mount_root: PathBuf,
    stream_fs_endpoint: String,
}

#[async_trait]
impl ResourceMounter for StreamFsMounter {
    async fn mount(
        &self,
        resource: &ResourceRef,
        runtime_key: &str,
    ) -> Result<Option<MountedResource>>;
}
```

Responsibilities:

1. recognize `ResourceRef::StreamFs` (or the equivalent split
   `ResourceSourceRef::StreamFs`)
2. validate that the requested mode is `SnapshotReadOnly`
3. validate that a revision is present
4. create or reuse a host-side mount path under a runtime-scoped mount root
5. invoke the stream-fs mount helper / daemon
6. return `MountedResource { host_path, mount_path, read_only: true }`

The mounter does **not** implement stream-fs semantics itself. It is a
translation layer from Fireline's resource contract to the existing
stream-fs package.

### 4.2 Host-side FUSE presentation

The FUSE path is the core of the spike. The mount lifecycle is:

1. prepare a local host directory such as
   `/tmp/fireline-stream-fs/<runtime_key>/<resource_id>`
2. mount the stream-fs workspace revision there through the existing
   package or a small wrapper around it
3. hand that directory back as `MountedResource.host_path`
4. let the existing runtime provider bind-mount it into the runtime at
   the requested `mount_path`

That keeps the runtime oblivious to stream-fs. Inside the runtime, the
agent just sees a normal filesystem path.

This is the whole point of the spike. If the only viable integration is
through a custom client library in the runtime, the experiment is much
less compelling.

### 4.3 Composition with discovery

The spike assumes the discovery-plane shape from the companion docs:

- Host discovery comes from `hosts:tenant-<id>`
- resource discovery comes from `resources:tenant-<id>`

That means stream-fs does **not** need its own catalog. Host A publishes:

```json
{
  "kind": "resource_published",
  "resource_id": "shared-workspace",
  "source_ref": {
    "kind": "stream_fs",
    "source_ref": "workspace:demo/shared-workspace",
    "revision": "rev-abc123",
    "mode": "snapshot_read_only"
  },
  "metadata": {
    "tags": ["workspace", "demo"]
  },
  "published_by": "host:laptop",
  "published_at_ms": 1775904000000
}
```

Host B discovers that event the same way it would discover S3, Git, or
`DurableStreamBlob`.

### 4.4 Composition with deployment topology

This spike sits cleanly inside the topology from
[`deployment-and-remote-handoff.md`](./deployment-and-remote-handoff.md):

- Durable Streams is already the shared hub
- Hosts already depend on it
- sandboxes already consume mounted resources through the provider path

So the stream-fs spike does not introduce a new top-level artifact. It
reuses the existing durable-streams deployment and tests whether one
additional mounted backend can ride on top.

## 5. The demo this spike should enable

The demo beat is intentionally simple:

1. On Host A, publish a `StreamFs` resource for a small shared
   workspace, pinned to revision `R`.
2. On Host B, provision a runtime with that resource mounted at
   `/workspace/shared`.
3. Inside the runtime on Host B, run a trivial read:
   `cat /workspace/shared/README.md`
4. Show that the bytes match the content at revision `R`.

This is "cross-Host collaborative file access" in the narrowest honest
form:

- the resource is shared across Hosts
- it comes from a collaborative filesystem substrate
- the consuming runtime accesses it as a normal path

It is **not** "live collaborative editing." The word collaborative here
belongs to the backing store, not to the demonstrated semantics of the
mount mode.

## 6. What the spike commit contains

The spike should fit in one isolated scaffolding commit and contain
only the pieces required to prove the path above.

### 6.1 Fireline-side scaffolding

- `StreamFsMounter` in `fireline-resources`
- enough resource-type expansion to recognize the `StreamFs` variant
- provider wiring so a runtime can receive the mounted path through the
  existing `ResourceMounter` contract
- a small mount-lifecycle helper that creates and tears down the host
  mount directory

### 6.2 Minimal integration wrapper

- a thin wrapper over the existing `packages/stream-fs` tooling or
  helper process
- explicit contract: mount `source_ref` at `revision` in
  read-only snapshot mode
- no attempt to expose live watchers or writable semantics

### 6.3 One proof path

One end-to-end proof is enough for the spike:

- publish a `StreamFs` resource
- discover it
- mount it
- read a known file from inside a launched runtime

That proof can be manual, a demo script, or a narrow automated test in a
later implementation lane. The proposal itself does not require the test
shape; it only constrains the feature shape.

## 7. What the spike commit does NOT contain

The spike explicitly excludes the following:

### 7.1 Live writable mode

No `StreamFsMode::LiveReadWrite` support.

Reason:

- it inherits the distributed-write risks called out in the evaluation
  doc
- it turns the spike from "can this mount cleanly?" into "what
  filesystem contract does Fireline expose?"

That is a different proposal.

### 7.2 Conflict resolution

No conflict handling policy, no optimistic concurrency contract, no
rename guarantees, no multi-writer safety story.

If the backing store already has semantics here, they remain behind the
backing store boundary and are not promoted into a Fireline contract by
the spike.

### 7.3 Cache invalidation and live coherence

No watcher propagation, no cache invalidation guarantees, no promise
that a second Host sees a first Host's writes "quickly enough."

Pinned snapshot read-only mode avoids the question entirely.

### 7.4 New discovery-plane design

No separate stream-fs registry, no new discovery service, no extra
artifact catalog. The spike must compose with the already-proposed
`resources:tenant-<id>` stream or it is proving the wrong architecture.

### 7.5 General stream-fs productization

No attempt to prove that stream-fs should become a default or preferred
Resources backend. The only thing the spike can prove is:

> a pinned, read-only stream-fs resource can be mounted cross-Host
> through the existing Fireline resource path.

## 8. Success and failure criteria

### Success

The spike is a success if all of the following are true:

- Fireline can represent a `StreamFs` resource in the existing
  resource-discovery shape
- a Host can mount a pinned revision as a normal host path
- the runtime provider can bind-mount that path without special cases
- an agent or test process inside the runtime can read files from that
  path successfully

### Failure

The spike should be considered a failure if any of the following are
true:

- stream-fs cannot expose a stable revision/snapshot identity
- the only viable integration path requires custom runtime-side client
  logic rather than a host-side mount
- the mount lifecycle is too fragile to fit the existing provider
  contract
- the FUSE path is operationally unrealistic for Fireline's target
  environments

A failed spike is still useful. It would answer the question cleanly and
let the project keep stream-fs parked as "interesting, but not a good
fit right now."

## 9. Recommendation

Run the spike only in the shape described here:

- `StreamFsMounter`
- host-side FUSE mount
- bind-mount into runtime
- pinned revision
- read-only snapshot mode

If that narrow path works, Fireline gets a credible demo of cross-Host
shared file access with one additional backend and no new discovery
plane.

If that narrow path does not work, the project should not expand scope
into live writable mode. The right next step would be to keep
`StreamFs` as a resource-discovery variant on paper while treating its
implementation as deferred.
