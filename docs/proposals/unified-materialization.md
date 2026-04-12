# Unified Materialization for Fireline Session Projections

## TL;DR

Fireline now has one explicit projection contract for durable-streams-backed
read models:

- `crates/fireline-session/src/projection.rs` defines the shared
  `StateEnvelope`, `StateHeaders`, `ChangeOperation`, `ControlKind`, and
  `StreamProjection` trait.
- `crates/fireline-session/src/state_materializer.rs` owns the single
  subscribe -> replay -> live-tail loop and fans decoded envelopes out to
  `Vec<Arc<dyn StreamProjection>>`.
- `SessionIndex`, `HostIndex`, and `ActiveTurnIndex` now implement
  `StreamProjection::apply()` directly instead of each carrying their own
  private envelope model and projection interface.

This keeps one stream reader per state stream, one protocol model for the
reader side, and one trait for in-memory projections.

## Problem

Before this change, Fireline already had a shared `StateMaterializer`, but the
abstraction stopped halfway.

- `state_materializer.rs` owned transport concerns: connect, replay from the
  beginning, live-tail, chunk parsing, and fanout.
- `session_index.rs`, `host_index.rs`, and `active_turn_index.rs` each
  reimplemented their own state-envelope handling, operation matching, row
  deserialization, delete semantics, and reset logic.
- The projection trait itself lived inside `state_materializer.rs`, which made
  the transport layer the de facto owner of the projection contract.

That meant new projections kept copying the same pattern:

1. Match on entity type.
2. Match on operation.
3. Deserialize the row.
4. Mutate local maps.
5. Clear local state on `reset`.

The code worked, but the shared projection boundary was implicit rather than
obvious.

## Implemented Design

### 1. Shared protocol model in `projection.rs`

`projection.rs` now defines the reader-side protocol model that Fireline
materializers consume:

```rust
pub trait StreamProjection: Send + Sync {
    fn apply(&self, envelope: &StateEnvelope) -> Result<()>;

    fn reset(&self) -> Result<()> {
        Ok(())
    }
}
```

The same module also defines the envelope types:

- `StateEnvelope`
- `StateHeaders`
- `ChangeOperation`
- `ControlKind`

This is the single reader-side shape that projections operate on.

### 2. `StateMaterializer` stays the transport/runtime

`StateMaterializer` still owns:

- building the durable-streams reader
- replaying from `Offset::Beginning`
- following the live tail
- parsing JSON-array chunks
- decoding each item into `StateEnvelope`
- classifying change vs control messages
- broadcasting each decoded envelope to all registered projections

That part of the design was already correct, so it stayed in place. The change
was to make its projection dependency explicit and reusable instead of private
to the file.

### 3. Indexes are now plain projections

`SessionIndex`, `HostIndex`, and `ActiveTurnIndex` each now implement
`StreamProjection` directly.

Each projection keeps its own narrow in-memory state and only owns the logic
specific to its entity families:

- `SessionIndex` projects `session` and `runtime_spec`
- `HostIndex` projects `runtime_spec`, `runtime_instance`, and
  `runtime_endpoints`
- `ActiveTurnIndex` projects `prompt_turn`

`ActiveTurnIndex` still includes runtime-local waiter coordination on top of
the projection state. That is acceptable for now because the durable-state
application path is still unified through `StreamProjection::apply()`.

## Alignment with Durable Streams `STATE-PROTOCOL`

The shared `StateEnvelope` matches the upstream protocol fields Fireline needs
from `packages/state/STATE-PROTOCOL.md`:

- change messages:
  - `type`
  - `key`
  - `value`
  - `old_value`
  - `headers.operation`
  - `headers.txid`
  - `headers.timestamp`
- control messages:
  - `headers.control`
  - `headers.offset`

The implementation now explicitly models those optional fields instead of
dropping them at deserialize time. That keeps Fireline aligned with the wire
format even when a given projection only uses `type`, `key`, `value`, and
`headers.operation` today.

`StateMaterializer` handles the protocol like this:

- change messages are fanned out to every projection via `apply()`
- `snapshot-start` and `snapshot-end` are observed passively
- `reset` clears each projection through `StreamProjection::reset()`

One important limit remains: Fireline observes `headers.offset` on reset but
does not yet rebuild the reader from that offset. This proposal only unifies
the projection abstraction; it does not add reset-seek recovery semantics.

## Migration from the Previous Layout

The old structure looked like this:

- `state_materializer.rs`: transport + private trait + private envelope types
- `session_index.rs`: local projection logic
- `host_index.rs`: local projection logic
- `active_turn_index.rs`: local projection logic + waiters

The new structure is:

- `projection.rs`: shared protocol types + shared `StreamProjection` trait
- `state_materializer.rs`: transport and fanout only
- `session_index.rs`: session projection
- `host_index.rs`: host projection
- `active_turn_index.rs`: active-turn projection plus waiters

This is a smaller refactor than a fully generic materialization framework, but
it is the correct cut for the codebase today:

- transport stays centralized
- protocol modeling is shared
- projection behavior is explicit
- each read model keeps only its entity-specific logic

## Why This Shape

The main reason not to over-generalize further is that Fireline already has a
working one-reader materializer runtime. The missing piece was not another
runtime abstraction; it was a shared contract that made the projections look
like projections.

This implementation is intentionally conservative:

- no new generic state container
- no erased typed-state framework
- no writer-side protocol unification in this commit
- no reset offset re-seek behavior

Those can be layered later if they become necessary. The immediate win is that
new in-memory indexes now have one obvious way to plug into the durable state
stream.
