# Stream-FS as a Fireline Resources Backend

> Status: exploratory input, not yet adopted
> Type: decision-framing doc
> Audience: maintainers or planning agents deciding whether `stream-fs` should influence Fireline's `Resources` primitive
> Source:
> - [`./managed-agents-mapping.md`](./managed-agents-mapping.md) — Fireline's anchor doc for the six managed-agent primitives
> - [`../execution/15-workspace-object.md`](../execution/15-workspace-object.md) — current Resources refactor plan
> - external review input:
>   - Durable Streams `stream-fs` spec at commit `71b3555684391d6314daea15a879777f620d8c76`: <https://github.com/durable-streams/durable-streams/blob/71b3555684391d6314daea15a879777f620d8c76/packages/stream-fs/SPEC.md>
>   - Durable Streams PR `#247`: <https://github.com/durable-streams/durable-streams/pull/247>

## Why this doc exists

Fireline's current substrate gap is not "shared workspaces" in the abstract. It is
the much smaller `Resources` primitive:

```text
[{source_ref, mount_path}]
```

The anchor doc is explicit: Fireline still needs a launch-time resource contract,
a `ResourceMounter` seam, and provider integration that materializes resources
before runtime start. See [`managed-agents-mapping.md`](./managed-agents-mapping.md)
§"5. Resources — Missing".

At the same time, Durable Streams now has an experimental `stream-fs` package: a
shared, durable, eventually consistent filesystem built on append-only streams.
This raises a legitimate planning question:

> Should Fireline use `stream-fs` as part of the Resources solution?

This doc answers that question narrowly.

## Executive summary

`stream-fs` is interesting for Fireline, but **not as the first implementation of
the Resources primitive**.

It is better understood as:

- **not** a replacement for `ResourceRef + ResourceMounter`
- **not** the v1 answer to "portable launch inputs"
- **possibly** a later `ResourceMounter` backend for shared or durable workspaces
- **much more compelling** if there is a viable path to presenting it as a normal
  host-side mount, especially via FUSE

The best first use, if explored at all, is:

- a **read-only, pinned snapshot** resource
- mounted on the host
- bind-mounted into the runtime at `mount_path`

The riskiest use is:

- a **live, writable, multi-agent shared mount**

That mode is conceptually attractive but asks Fireline to inherit distributed
filesystem semantics it does not currently need for closing the `Resources`
primitive.

## Fireline's actual gap

Per the anchor doc, Fireline still needs:

- `resources: Vec<ResourceRef>` on `CreateRuntimeSpec`
- a `ResourceMounter` trait
- at least one local-path implementation
- at least one network-fetched implementation such as git or S3
- a documented contract for how mounters interact with provider startup
- one end-to-end proof that a runtime reads mounted material from a non-local
  resource

See:

- [`managed-agents-mapping.md`](./managed-agents-mapping.md#5-resources--missing)
- [`managed-agents-mapping.md`](./managed-agents-mapping.md#resources--acceptance-bar)
- [`../execution/15-workspace-object.md`](../execution/15-workspace-object.md)

This means the planning sequence should still begin with the generic substrate
shape:

1. `ResourceRef`
2. `ResourceMounter`
3. provider launch integration
4. one local and one remote resource proof

Only after that should Fireline decide whether `stream-fs` is a resource kind
worth adding.

## What `stream-fs` is

The reviewed `stream-fs` package is a shared filesystem abstraction built over:

- one metadata stream for path-level inserts, updates, and deletes
- one content stream per file for `init`, `replace`, and `patch` events
- in-memory materialized views inside each client
- SSE-based `watch()` support for change propagation

The useful mental model is:

- it behaves like a durable, replayable, collaborative filesystem
- it does **not** behave like a strongly consistent POSIX filesystem

From the review:

- the package is credible as a PoC
- the test surface is broad and passes in the monorepo project runner
- the semantics are weaker than the spec language in several places
- the implementation is eventually consistent and practically last-writer-wins
  under contention
- move / rename behavior is multi-step, not atomic

That makes it promising as a collaboration substrate, but not something Fireline
should casually expose as if it were a strong local disk.

## Where `stream-fs` could fit

There are three distinct roles `stream-fs` could play in Fireline.

### 1. As a direct replacement for Resources

This is the wrong framing.

`stream-fs` does not remove the need for:

- `ResourceRef`
- `ResourceMounter`
- runtime-start materialization
- a documented provider contract

At best, `stream-fs` would be **one source kind** under that contract.

### 2. As a later resource backend

This is the most plausible framing.

Fireline could add a resource kind such as:

```ts
type ResourceRef =
  | { kind: "local_path"; source_ref: string; mount_path: string }
  | { kind: "git"; source_ref: string; mount_path: string }
  | { kind: "s3"; source_ref: string; mount_path: string }
  | { kind: "stream_fs"; source_ref: string; mount_path: string; revision?: string }
```

Under that model:

- the generic Resources primitive stays simple
- `stream-fs` becomes a backend-specific `ResourceMounter`
- downstream products opt into it only when they need shared durable state

### 3. As a shared live workspace layer

This is strategically interesting, but materially bigger than the current
Resources gap.

Once Fireline lets multiple runtimes mount the same live `stream-fs` workspace,
it is implicitly making choices about:

- write ownership
- conflict semantics
- rename behavior
- crash recovery
- cache invalidation
- replay and reproducibility

Those are distributed-filesystem questions, not just launch-spec questions.

## Benefits if Fireline uses `stream-fs`

### Shared mutable working state

Multiple runtimes could observe and mutate the same durable filesystem-shaped
state instead of each starting from an isolated clone or downloaded archive.

### Durable replay

A new runtime can reconstruct the working set from stream history. This fits
Fireline's existing comfort with append-only logs and replayed state.

### Live collaboration

`watch()` plus SSE makes it plausible to keep peers or UIs up to date with file
changes without inventing a second synchronization channel.

### Incremental text updates

For text-heavy workloads, patch-based writes can be more efficient than
re-uploading full directory trees.

### Conceptual alignment with Fireline's substrate

At a high level, `stream-fs` matches Fireline's durable-stream style better than
inventing a Fireline-owned workspace database or catalog object.

## Tradeoffs and risks

### It does not close the immediate Resources gap

The first planning mistake to avoid is treating `stream-fs` as the missing
primitive. It is not. Fireline still needs the generic Resources surface first.

### Weak filesystem semantics

`stream-fs` is eventually consistent, not linearizable. In the reviewed PR:

- stale-write protection is local, not server-enforced
- concurrent writers can still race
- rename / move is not atomic
- delete cleanup is best-effort rather than a hard guarantee

This matters much more once Fireline presents it as a normal filesystem mount.

### Reproducibility tension

The Resources primitive is fundamentally about **portable launch inputs**.
Git SHAs and object-store versions are naturally reproducible. A live shared
filesystem is not, unless Fireline introduces explicit snapshot or revision
pinning.

### Operational complexity

Using `stream-fs` introduces concerns Fireline does not need for `LocalPath` or
`GitRemote`:

- auth and isolation by stream prefix
- mount lifecycle and cleanup
- cache and reconnect behavior
- garbage collection of old content streams or snapshots
- durability and availability assumptions for the backing durable-streams server

### Tooling expectations

If an agent sees a normal filesystem path, it will assume ordinary filesystem
semantics. Any mismatch between POSIX expectations and actual `stream-fs`
behavior becomes a product risk.

## The FUSE pathway

The existence of a viable FUSE path changes the answer significantly.

Without FUSE or an equivalent mount presentation, `stream-fs` is mostly a
library or protocol. That makes it awkward for Fireline, because the runtime
usually wants a real local path.

With FUSE, `stream-fs` becomes much more plausible as a `ResourceMounter`
implementation:

1. materialize or mount the `stream-fs` resource on the host
2. bind-mount that path into the runtime at `mount_path`
3. let the agent and tools interact with an ordinary filesystem path

### What FUSE improves

- compatibility with editors, shells, compilers, and git
- cleaner provider integration
- less Fireline-specific client logic inside the runtime
- a much more natural mapping onto the existing Resources primitive

### What FUSE does not improve

FUSE does **not** strengthen the underlying semantics. It only changes the
presentation layer.

If Fireline exposes `stream-fs` through FUSE, it must still decide:

- whether the mount is read-only or writable
- whether the mount is pinned to a snapshot or live
- how to surface unsupported or weak operations
- how to document the real contract behind the POSIX-looking path

### Recommended first FUSE mode

If Fireline explores this, the first mode should be:

- host-side mount
- read-only
- pinned snapshot or revision
- bind-mounted into the runtime

That gives most of the ergonomic upside of FUSE without committing Fireline to
live multi-writer semantics.

## Work required to make this viable

The work splits into Fireline work and `stream-fs`-specific work.

### Fireline work

#### Phase 0: land the generic Resources primitive

Before any `stream-fs` integration:

- add `resources` to `CreateRuntimeSpec`
- define `ResourceRef`
- define `ResourceMounter`
- wire mounters into `RuntimeProvider::start()`
- ship `LocalPathMounter`
- ship one network-fetched mounter such as `GitRemoteMounter`

This is still the critical path.

#### Phase 1: define a `stream-fs` resource kind

Add a resource kind with explicit semantics, for example:

```ts
type StreamFsResourceRef = {
  kind: "stream_fs";
  source_ref: string;
  mount_path: string;
  revision?: string;
  mode?: "snapshot_ro" | "live_ro" | "live_rw";
}
```

The important field is `revision` or equivalent. Without that, Fireline cannot
offer reproducible launch inputs.

#### Phase 2: implement a `StreamFsMounter`

The mounter must:

- resolve the source reference
- optionally resolve a pinned revision
- mount or materialize a local filesystem view
- return the mount/bind information the provider needs

#### Phase 3: provider integration

Providers must be able to:

- ensure the mount exists before runtime launch
- keep it alive for the runtime lifetime
- tear it down cleanly when the runtime stops

### `stream-fs` work

#### Snapshot or revision identity

This is the largest missing capability for Fireline use.

Fireline needs a stable answer to:

> What exact filesystem state did this runtime launch with?

If `stream-fs` cannot answer that with a stable revision or snapshot ID, it is a
poor fit for Resources v1.

#### Stronger concurrency model, if writable

For any live writable mode, `stream-fs` would need:

- clearer server-enforced preconditions
- better conflict handling
- more robust move / rename behavior

Without those, Fireline should treat writable live mounts as experimental.

#### Mount-capable implementation

If FUSE is the chosen path, the implementation needs:

- a host-side daemon or mount helper
- local caching
- reconnect behavior
- invalidation strategy
- teardown behavior

#### Security and isolation model

Fireline would need a clear contract for:

- auth to the durable-streams backend
- tenant and prefix isolation
- secret handling for source references

## Recommended planning posture

Planning should assume the following:

### 1. `stream-fs` is not the next Resources slice

The next slice should still be:

- `ResourceRef`
- `ResourceMounter`
- `LocalPathMounter`
- `GitRemoteMounter` or `S3Mounter`

That is the direct route to closing the managed-agent gap.

### 2. `stream-fs` is worth a later, narrow spike

The best spike question is:

> Can Fireline mount a pinned `stream-fs` snapshot as a normal read-only path?

This is deliberately narrower than:

> Can Fireline support live shared writable workspaces?

The first question tests the ergonomic upside without inheriting all of the
distributed-write risks.

### 3. FUSE makes this much more interesting

If there is a real path to presenting `stream-fs` as a regular host-side mount,
then `stream-fs` deserves attention as a plausible later `ResourceMounter`.

If there is no mount presentation, its fit for Fireline is much weaker.

## Suggested planning output

If a planning agent uses this doc, the expected output should be one of:

### Option A: defer

Conclusion:

- land generic Resources first
- do not spend near-term time on `stream-fs`

This is the safest default.

### Option B: narrow spike

Conclusion:

- keep generic Resources on the critical path
- run a short spike on `stream-fs` as a pinned read-only mounted resource

Spike questions:

- can `stream-fs` identify immutable revisions or snapshots?
- can it be presented as a normal host-side mount?
- can a runtime read mounted data without any custom client code?
- what are the provider lifecycle hooks needed?

### Option C: strategic investment

Conclusion:

- Fireline wants a durable shared-workspace substrate badly enough to invest
  beyond Resources v1

This should only be chosen if a downstream product genuinely needs live shared
mutable workspaces, not just portable launch inputs.

## Open questions for planning

1. Does `stream-fs` have, or can it cheaply gain, a pinned snapshot identity?
2. Is the FUSE path host-side and operationally realistic for Fireline's target
   providers?
3. Does Fireline actually need live shared writable workspaces, or only
   reproducible mounted inputs?
4. Should `stream-fs` be modeled as a read-only snapshot source first, with live
   writable mode explicitly deferred?
5. If writable mode is ever attempted, what concurrency contract will Fireline
   expose to users?

## Recommendation

Treat `stream-fs` as a **promising later backend**, not as the first answer to
the Resources primitive.

The recommended sequence is:

1. close the generic Resources gap
2. ship `LocalPathMounter`
3. ship `GitRemoteMounter` or `S3Mounter`
4. only then evaluate `stream-fs` as a mounted backend
5. if evaluated, start with **read-only pinned snapshot mount**

That path preserves Fireline's current substrate simplification while keeping the
door open to a stronger shared-workspace story later.
