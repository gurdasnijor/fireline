# Resource discovery

## 1. TL;DR

- Durable Streams can be the discovery plane for **resources** the same
  way it can be the discovery plane for Hosts and runtimes: publishing
  a small event makes a file, mount, or artifact discoverable across the
  deployment.
- The backing store is no longer the catalog. S3, Docker volumes, local
  paths, Git repos, OCI layers, HTTP URLs, durable-stream blobs, and
  `StreamFs` mounts all become one logical class:
  **discoverable resources**.
- A consumer does not need a per-Host file-helper REST API to ask
  "which machine has this file?" or "where does this artifact live?" It
  replays `resources:tenant-<id>`, projects a `ResourceIndex`, and
  resolves from there.
- This turns `fireline sync-to-remote` from a bespoke migration command
  into a special case of a more general pattern: **publish resource,
  discover resource, fetch resource**.
- Together with the companion
  [`cross-host-discovery.md`](./cross-host-discovery.md) proposal, this
  makes Durable Streams the universal discovery plane for Fireline
  deployments.

## 2. The insight

The cross-host discovery insight applies one layer down almost
unchanged: **files, mounts, and artifacts are just another discoverable
object class**. The mechanism is identical to Host discovery. A small
append-only event says "resource X exists, here is its backing store,
here is its metadata, here is who published it." Readers replay that
stream into an index and look resources up by id, tag, or other
metadata. The only thing that changes is the schema of the event, not
the architecture.

## 3. The stream shape

The discovery stream is tenant-scoped:

```text
Stream: resources:tenant-<tenant_id>
```

The first cut uses three envelope families:

```rust
resource_published {
    resource_id:      String,   // stable, URL-safe, tenant-unique
    source_ref:       ResourceSourceRef,
    metadata:         ResourceMetadata,
    published_by:     HostId,
    published_at_ms:  i64,
}

resource_unpublished {
    resource_id:         String,
    reason:              String,
    unpublished_at_ms:   i64,
}

resource_updated {
    resource_id:      String,
    new_metadata:     ResourceMetadataPatch,
    updated_at_ms:    i64,
}
```

### Why one stream per tenant

- It matches the companion Host discovery proposal's tenant-scoped
  stream design.
- It avoids a second registry or catalog service.
- It keeps replay and authorization boundaries aligned with the rest of
  the deployment story: session state, Host discovery, and resource
  discovery all live behind the same durable-streams ACL boundary.

### Why three event types are enough for M1

- `resource_published` creates the discoverable record.
- `resource_updated` mutates metadata without changing identity.
- `resource_unpublished` removes the record from live discovery.

Notably absent in M1:

- ownership transfer events
- rename events
- ACL mutation events
- content-addressable aliasing or dedup markers

Those are useful later, but they are not required to make the durable
stream the source of truth for resource discovery.

### Resource metadata

`metadata` is intentionally open-ended, but the common keys should be
documented up front so independently written publishers converge on one
shape:

```rust
pub struct ResourceMetadata {
    pub size_bytes: Option<u64>,
    pub mime_type: Option<String>,
    pub content_hash: Option<String>,
    pub tags: Vec<String>,
    pub permissions: Option<serde_json::Value>,
    pub description: Option<String>,
}
```

The stream does not need to understand these fields semantically. They
exist so projections and UIs can filter and display resources without
dereferencing the backing store first.

## 4. The `ResourceSourceRef` enum

This proposal deliberately factors discovery into two layers:

1. **Where the bytes live**
2. **Where the consumer wants them mounted**

That means the logical type is:

```rust
pub enum ResourceSourceRef {
    LocalPath {
        host_id: HostId,
        path: PathBuf,
    },
    S3 {
        bucket: String,
        key: String,
        region: String,
        endpoint_url: Option<String>,
    },
    Gcs {
        bucket: String,
        key: String,
    },
    DockerVolume {
        host_id: HostId,
        volume_name: String,
        path_within_volume: PathBuf,
    },
    DurableStreamBlob {
        stream: String,
        key: String,
    },
    StreamFs {
        source_ref: String,
        revision: Option<String>,
        mode: StreamFsMode,
    },
    OciImageLayer {
        image: String,
        path: PathBuf,
    },
    GitRepo {
        url: String,
        r#ref: String,
        path: PathBuf,
    },
    HttpUrl {
        url: String,
        headers: Option<HashMap<String, String>>,
    },
}

pub enum StreamFsMode {
    SnapshotReadOnly,
    LiveReadOnly,
    LiveReadWrite,
}
```

The published resource then combines that source with a mount intent:

```rust
pub struct PublishedResourceRef {
    pub source_ref: ResourceSourceRef,
    pub mount_path: PathBuf,
    pub read_only: bool,
}
```

### Important current-code drift

Today's TS and Rust surfaces do **not** yet expose a separate
`ResourceSourceRef`. They flatten source information and `mount_path`
into a single `ResourceRef`:

- [`packages/client/src/core/resource.ts`](../../packages/client/src/core/resource.ts)
- [`crates/fireline-resources/src/mounter.rs`](../../crates/fireline-resources/src/mounter.rs)

This proposal treats `ResourceSourceRef` as the correct logical split
for discovery and expects M2 to normalize the API accordingly. If the
project wants to preserve the flat external shape for compatibility, it
can still derive `source_ref` internally from that union; the
discovery-plane semantics stay the same.

### Variant responsibilities

| Variant | Who can publish it | Who resolves it | Reachability assumption |
|---|---|---|---|
| `LocalPath` | A Host that can prove the path exists on its own filesystem | `LocalPathMounter` on the same Host, or a cross-Host file fetcher on another Host | Local fast path only if `host_id == self`; remote use requires the publishing Host to remain reachable |
| `S3` | Any Host or CLI with S3 object metadata and permission to name the object | `S3Mounter` / S3-backed fetcher | Consumer Host must reach the S3 endpoint and hold credentials |
| `Gcs` | Any Host or CLI with GCS object metadata and permission to name the object | `GcsMounter` / GCS-backed fetcher | Consumer Host must reach GCS and hold credentials |
| `DockerVolume` | A Host that owns the named volume | Docker-volume mounter on that same Host | Not portable across Hosts unless another Host can attach the same named volume |
| `DurableStreamBlob` | Any Host or CLI that can write the blob into durable-streams storage | `DurableStreamMounter` / stream reader | Reachable from any Host that can reach the durable-streams service |
| `StreamFs` | Any Host or CLI that can name a stream-fs source and optionally pin a revision | `StreamFsMounter` | Consumer Host must be able to mount or materialize the referenced stream-fs source, ideally pinned to a revision |
| `OciImageLayer` | Any Host or CLI that can refer to an image digest or tag | OCI-layer fetcher / extractor | Consumer Host must reach the registry or have the image cached locally |
| `GitRepo` | Any Host or CLI that can name a repo/ref/path | `GitRemoteMounter` or fetcher | Consumer Host must reach the Git remote and hold credentials if needed |
| `HttpUrl` | Any Host or CLI that can name a URL | HTTP fetcher | Consumer Host must reach the URL; optional headers are hints, not an auth solution by themselves |

### Notes on specific variants

#### `LocalPath`

This is the most important variant for replacing the dead file-helper
REST API. `LocalPath` is no longer "magic local state hidden behind one
process." It becomes an explicitly published fact:

> Host X says resource `design-docs` lives at `/srv/workspaces/docs`.

Consumers then decide whether they can use that fact:

- same Host: mount directly
- different Host: fetch via ACP fs or re-publish as a
  `DurableStreamBlob`

#### `DockerVolume`

This variant exists because "bytes live in a Docker volume attached to
Host X" is a distinct operational fact from "bytes live at a path on
Host X." The discoverability mechanism is the same even if the mount
implementation differs.

#### `DurableStreamBlob`

This is the cleanest cross-Host transport. It is the direct
generalization of `sync-to-remote`: the bytes already live in the
shared durable-streams service, so any consumer can resolve them
without going back to the publishing Host.

#### `StreamFs`

`StreamFs` is a first-class discovery variant, but not a first-class
implementation priority. It should be treated as a later narrow
resource kind whose full tradeoffs are already captured in
[`../explorations/stream-fs-resources-evaluation.md`](../explorations/stream-fs-resources-evaluation.md).
For discovery purposes it behaves like any other source ref: publish a
small event naming the source, optionally pin a revision, and let a
`StreamFsMounter` decide whether it can materialize or mount it on the
consumer Host.

#### `OciImageLayer`

This matters less for editable workspaces and more for static assets,
tool bundles, and baked-in example corpora. It also gives the
microsandbox / OCI deployment story a discovery-plane analogue:
artifacts already inside container images become discoverable without a
second catalog.

## 5. The projection: `ResourceIndex`

Readers replay the tenant's resource stream into an in-memory
projection:

```rust
pub type ResourceId = String;
pub type HostId = String;

pub struct ResourceIndex {
    resources: HashMap<ResourceId, ResourceEntry>,
}

pub struct ResourceEntry {
    resource_id: ResourceId,
    source_ref: ResourceSourceRef,
    metadata: ResourceMetadata,
    published_by: HostId,
    first_seen_ms: i64,
    last_updated_ms: i64,
}

impl ResourceIndex {
    pub fn lookup(&self, id: &ResourceId) -> Option<&ResourceEntry>;
    pub fn list(&self) -> impl Iterator<Item = &ResourceEntry>;
    pub fn list_by_tag(&self, tag: &str) -> impl Iterator<Item = &ResourceEntry>;
}
```

### Projection rules

- `resource_published`
  inserts the record if absent; if the same `resource_id` already
  exists, treat it as an error unless the event is byte-for-byte
  identical to the existing source and publisher.
- `resource_updated`
  merges metadata into the existing row and updates
  `last_updated_ms`.
- `resource_unpublished`
  removes the row from the live projection.

### Merge semantics for `resource_updated`

The update event should be metadata-only. That means:

- `resource_id` is immutable
- `source_ref` is immutable
- `published_by` is immutable
- metadata keys merge in stream order

The projection never "partially updates" the source location. If the
underlying backing store changes, the publisher must create a new
resource id and unpublish the old one.

### Why project instead of querying the stream every time

Because discovery is query-heavy and event-light relative to the size of
the backing bytes. The stream is the source of truth, but the index is
the query surface:

- list every published artifact tagged `dataset`
- look up resource `workspace-snapshot`
- show all resources published by Host `host-demo-us-west-2`

Those are projection reads, not raw stream scans.

## 6. Consumer flow: from discovery to mount

The consumer flow is intentionally variant-dispatch plus fetch:

1. Subscribe to `resources:tenant-<id>`.
2. Replay it into `ResourceIndex`.
3. Look up a resource by id, tag, publisher, or some richer filter.
4. Read the `ResourceSourceRef`.
5. Dispatch to the appropriate mounter or fetcher.
6. Hand the mounted path or fetched bytes to the agent, tool, or
   sandbox provider.

### Variant-specific resolution flow

- `LocalPath { host_id: self, path }`
  Fast path. Resolve through the local filesystem and mount directly.
- `LocalPath { host_id: other, path }`
  Reach back to the publishing Host. In the companion Host discovery
  proposal that means: discover Host `other`, open ACP to it, and issue
  `fs/read_text_file` or a higher-level file transfer method. For large
  artifacts, the better move is usually to re-publish as
  `DurableStreamBlob`.
- `S3`
  Use an S3-backed mounter or raw object fetcher.
- `Gcs`
  Use a GCS-backed mounter or raw object fetcher.
- `DurableStreamBlob`
  Read the named blob from durable-streams and materialize locally.
- `StreamFs`
  Resolve through a `StreamFsMounter`, present it as a host-side FUSE
  mount, and bind-mount that into the runtime at `mount_path`. When a
  `revision` is present, pin to it for reproducibility.
- `OciImageLayer`
  Pull and extract the image layer or reuse a locally cached image.
- `GitRepo`
  Clone / fetch the repo, checkout the ref, and read the path.
- `HttpUrl`
  Perform an HTTP GET, optionally with the hinted headers.

### The important architectural boundary

The resource stream answers:

> What is this thing called, where does it live, and what metadata do I
> know about it?

It does **not** answer:

> Can this particular Host actually reach the bytes right now?

Reachability remains the responsibility of the concrete fetcher. This is
the same separation the companion Host discovery proposal makes between
"a Host is discoverable" and "a Host's ACP URL is reachable from here."

## 7. Publisher flow: from local file to discoverable resource

The publish path should exist as both library API and CLI wrapper.

### Host/library API

```rust
pub trait ResourcePublisher {
    async fn publish_resource(
        &self,
        id: ResourceId,
        source_ref: ResourceSourceRef,
        metadata: ResourceMetadata,
    ) -> Result<()>;

    async fn update_resource(
        &self,
        id: &ResourceId,
        patch: ResourceMetadataPatch,
    ) -> Result<()>;

    async fn unpublish_resource(
        &self,
        id: &ResourceId,
        reason: &str,
    ) -> Result<()>;
}
```

### CLI wrapper

```text
fireline publish-resource <path-or-ref> --id=<name> [--tag <tag>]...
```

The CLI is not a separate writer model. It is a convenience wrapper over
the same publication contract:

1. inspect the local input
2. choose or synthesize a `ResourceSourceRef`
3. emit `resource_published`
4. optionally upload or transform bytes first when the chosen source
   needs it

### Reframing `sync-to-remote`

The deployment proposal's `fireline sync-to-remote` sketch becomes a
special case:

- old model: sync is a bespoke command that moves files to some remote
  place
- new model: sync is **publish a resource** and let remote consumers
  discover and fetch it

For example:

- local directory -> upload to durable-streams blob -> publish
  `DurableStreamBlob`
- local path that should stay host-local -> publish `LocalPath`
- Git repository -> publish `GitRepo`

The durable stream is the naming and discovery layer. The backing store
still carries the bytes.

## 8. The `ResourceRegistry` trait

The consumer-side abstraction should mirror `PeerRegistry` closely so
discovery remains a swappable satisfier rather than a baked-in storage
choice.

```rust
#[async_trait]
pub trait ResourceRegistry: Send + Sync {
    async fn lookup(&self, id: &ResourceId) -> Result<Option<ResourceEntry>>;
    async fn list(&self) -> Result<Vec<ResourceEntry>>;
    async fn list_by_tag(&self, tag: &str) -> Result<Vec<ResourceEntry>>;
    async fn subscribe(&self, watcher: Box<dyn ResourceWatcher>) -> Result<Subscription>;
}

pub struct StreamResourceRegistry {
    stream_url: String,
    tenant_id: String,
    index: Arc<RwLock<ResourceIndex>>,
    subscription: /* durable-streams live subscription */,
}
```

### Where it should live

This proposal recommends:

- `ResourceRegistry` trait in `fireline-resources`
- `StreamResourceRegistry` implementation in `fireline-resources`

Reasoning:

- the abstraction is about the **Resources primitive**, not the Tools
  primitive
- the concrete entry type depends on `ResourceSourceRef` and
  `ResourceMetadata`
- the existing crate already owns `ResourceRef`, `ResourceMounter`, and
  `FsBackendComponent`

That said, the trait should be intentionally shaped like
`PeerRegistry`, and a later refactor could move both into a lower
shared "discovery" crate if the project wants one universal registry
layer.

### API change called out explicitly

M2 needs a public API update in both TS and Rust:

- current TS: flat `ResourceRef` union in
  [`packages/client/src/core/resource.ts`](../../packages/client/src/core/resource.ts)
- current Rust: flat `ResourceRef` enum in
  [`crates/fireline-resources/src/mounter.rs`](../../crates/fireline-resources/src/mounter.rs)

To support discovery cleanly, both surfaces need:

- the new backing-store variants
- a stable `resource_id`
- either an explicit `ResourceSourceRef` type or an equivalent internal
  factoring that preserves the same semantics

## 9. Invariants for TLA

The future `verification/spec/deployment_discovery.tla` should check the
resource side with the same style as the Host discovery companion spec.

- **ResourcePublishedIsEventuallyDiscoverable**
  If resource X is published at offset N, any reader that has replayed
  past N observes X in its `ResourceIndex`.
- **ResourceUnpublishedIsEventuallyInvisible**
  If resource X is unpublished at offset M, any reader that has replayed
  past M observes X as absent from live discovery.
- **ResourceUpdateMergesMetadata**
  `resource_updated` events merge metadata in stream order; later events
  overwrite earlier keys but do not rewrite `source_ref`.
- **PublisherOwnsUnpublish**
  Only the original publisher's `host_id` may emit
  `resource_unpublished` for a resource in M1. Ownership transfer is a
  future extension.
- **SourceRefIsImmutableAfterPublish**
  Once resource X is published, its `source_ref` never changes. Changing
  backing stores means publishing a new resource id and unpublishing the
  old one.

Two additional invariants are worth carrying forward even though they
are not strictly required by the dispatch:

- **ResourceIdIsUniqueWithinTenant**
  A reader never projects two live records with the same `resource_id`.
- **UnpublishedResourceCannotBeUpdated**
  A `resource_updated` event for a resource that is absent from the
  projection is ignored or rejected; it must not resurrect the row
  implicitly.

## 10. Comparison to traditional resource catalogs

| Approach | What it gives you | What it costs you | Why the stream-backed approach is better here |
|---|---|---|---|
| Per-Host file helper REST API | Ad hoc file browsing from one machine | Extra routes, extra auth surface, no shared discovery state | Dead in the current repo and fundamentally host-local |
| Kubernetes ConfigMaps / Secrets as catalog | Cluster-native lookup for small config blobs | Only works inside k8s, poor fit for large artifacts or multi-store references | Fireline needs one mechanism that works on laptop, VM, and cluster |
| S3 bucket listing as the catalog | Backing bytes and discovery in one place | Works only for S3-backed resources; no answer for local paths, git, OCI, volumes | Resource discovery must be backing-store agnostic |
| Artifact registry per type | Good UX for one artifact class | Every new artifact class needs another service or schema | Durable streams already exists and already spans the deployment |
| Durable-streams-backed resource discovery | One append-only discovery plane for every resource class | Requires projections and explicit reachability handling | Same infrastructure as state, same failure mode, same tenant boundary |

The important win is not "streams are a better object store." They are
not. The win is:

> one shared naming and discovery mechanism for all stores.

## 11. What this replaces

This proposal makes several old or half-built paths obsolete:

- The dead `/api/v1/files/*` REST helper investigated in
  [`../investigations/file-helper-api-usage.md`](../investigations/file-helper-api-usage.md)
- The `connections.rs` TODO stub that tried to map `connection_id ->
  cwd` for that REST helper
- `fireline sync-to-remote` as a bespoke file-sync command; it becomes
  publish + discover + fetch
- Any "which Host has this file?" logic that depends on local
  per-machine directories or other non-stream state

The replacement is not "add another API." The replacement is:

- publish a resource event
- project it from the stream
- dispatch to the right fetcher

## 12. What this does NOT solve

- **Access control**
  Publishing a resource does not grant every reader permission to fetch
  it. ACLs compose on top, likely through durable-streams ACLs plus
  per-resource policy and the SecretsInjectionComponent work.
- **Physical bytes transport**
  Discovery tells a consumer where the bytes live. It does not move
  them by itself. `LocalPath` on another Host still needs ACP fs or
  some higher-level transfer path. `S3` still needs S3 credentials.
- **Content-addressable dedup**
  Two resource ids can legitimately point at byte-identical artifacts.
  Dedup belongs in the backing store layer, not the discovery layer.
- **Real-time mutation**
  Publishing a local path does not create a live file watcher. If the
  underlying bytes change, the publisher must emit `resource_updated`
  or publish a new resource id.
- **Safe live-writable stream-fs semantics**
  `StreamFsMode::LiveReadWrite` inherits the distributed-write and
  consistency risks called out in
  [`../explorations/stream-fs-resources-evaluation.md`](../explorations/stream-fs-resources-evaluation.md).
  Post-demo work can explore that path, but M1 should treat pinned
  snapshot or read-only modes as the realistic starting point.

## 13. Milestones

- **M1**: write this proposal doc
- **M2**: expand the TS and Rust resource surfaces with the new backing
  store variants and a stable resource identity
- **M3**: implement `StreamResourceRegistry` in `fireline-resources`
- **M4**: wire Host-side `publish_resource` support into the runtime /
  CLI path that already knows how to talk to durable streams
- **M5**: delete the dead file-helper REST stubs
- **M6**: add `fireline publish-resource`
- **M7**: ship resolver implementations in priority order:
  `DurableStreamBlob`, `GitRepo`, `S3`, `Gcs`, `HttpUrl`,
  `OciImageLayer`, `DockerVolume`, `StreamFs`
- **M8**: update
  [`deployment-and-remote-handoff.md`](./deployment-and-remote-handoff.md)
  Section 4 to reframe file sync as publish/discover

## 14. Cross-reference to cross-host discovery

This proposal and the companion
[`cross-host-discovery.md`](./cross-host-discovery.md) document the
same mechanism applied to different object classes:

- `hosts:tenant-<id>` discovers Hosts and runtimes
- `resources:tenant-<id>` discovers files, mounts, and artifacts

Together they define **durable-streams as universal discovery**:

- one discovery plane
- one replay model
- one projection pattern
- one future TLA surface in `deployment_discovery.tla`

The long-term UX implication is that a single `fireline sync` command
could unify:

- host bootstrap / announce
- resource publication
- durable-stream-backed handoff metadata

That is the structural payoff of the user's insight: publishing a small
event is the universal act that makes a named thing discoverable in a
Fireline deployment.
