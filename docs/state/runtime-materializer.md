# Runtime Materializer

## Purpose

Fireline's runtime sometimes needs small in-memory lookups derived from its own
durable state stream.

Examples:

- `SessionIndex` for `session/load`
- `ActiveTurnIndex` for peer-call lineage lookup
- future narrow runtime-side lookups for reconnect or topology coordination

The right pattern is:

- one shared durable-streams subscriber / preload loop
- many small projections fed from that loop

The wrong pattern is:

- one ad hoc SSE reader per index
- or a revived Rust `StreamDb` that becomes a second canonical read model

## Why this exists

Without a shared materializer, Fireline will keep copy-pasting:

- stream read setup
- replay + live-follow loops
- preload barriers
- chunk decoding
- envelope filtering

for every new operational lookup.

That duplication is not architectural clarity. It is just drift waiting to
happen.

## Architectural rule

Rust may maintain **runtime-local materialized projections** over the durable
state stream when the runtime itself needs them for protocol coordination.

Examples:

- lookup a `SessionRecord` by `sessionId`
- lookup the currently active prompt turn for a bound ACP session
- later, lookup durable child-session edges for reconnect helpers

Those projections are:

- in-memory only
- replayable from the durable stream
- replaceable
- operational, not product-facing

They do not outrank the durable log.

## What this is not

This is not a return to the old Rust `StreamDb` model.

Fireline should not reintroduce:

- a Rust-owned canonical entity schema
- a general Rust query API
- a second consumer database parallel to `@fireline/state`
- broad Rust collection snapshots as a first-class product surface

TypeScript still owns the consumer schema and read model.

Rust only materializes the narrow operational views it needs to run the
runtime correctly.

## Proposed shape

Fireline should have one small runtime-side materializer host with:

- one durable stream reader
- one preload barrier
- one fanout path to registered projections

Conceptually:

```rust
trait StateProjection: Send + Sync {
    fn apply(&self, event: &serde_json::Value) -> anyhow::Result<()>;
}

struct RuntimeMaterializer {
    projections: Vec<Arc<dyn StateProjection>>,
}
```

The important part is the shape, not the exact trait.

Each projection should stay very small and explicit.

Examples:

- `SessionIndex`
- `ActiveTurnIndex`
- future `ChildSessionEdgeIndex`

## Data flow

The intended flow is one-way:

```text
ACP traffic
  -> Fireline trace/state producer
  -> durable state stream
  -> runtime materializer
  -> narrow in-memory projections
  -> runtime coordination logic
```

That means:

- tracer/state producer writes durable facts
- projections consume durable facts
- peer tools / load coordination read projections

The tracer should not write side-channel mutable state directly.

## Relationship to `@fireline/state`

`@fireline/state` remains the real consumer/materialization layer.

It owns:

- the durable schema
- local consumer DB materialization
- query helpers
- sink compositions

The runtime materializer exists only because the runtime itself must sometimes
answer questions like:

- "does this session exist?"
- "what is the active turn for this bound session?"

without becoming a general query server.

## Current implication

`SessionIndex` is already the first example of this pattern.

The next direct application is replacing `LineageTracker` with an
`ActiveTurnIndex` materialized from durable `prompt_turn` rows.

That is a better design because:

- it keeps data flow one-way
- it removes shared mutable side channels from the tracer path
- it makes peer lineage a join over durable state, not an extra tracker

## Scope boundary

This pattern is worth introducing now because it avoids repeating the same
subscriber/index boilerplate every time Fireline needs another small runtime
lookup.

It is not a reason to rebuild a full Rust-side state database.
