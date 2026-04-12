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
- The steady-state projection model is `SessionIndex` for agent-plane session
  state, `HostIndex` for infrastructure-plane host state, and ACP-keyed
  prompt-request / permission / tool-call projections that use
  `(SessionId, RequestId)` and `(SessionId, ToolCallId)` rather than synthetic
  ids. `ActiveTurnIndex` remains transitional compatibility scaffolding and is
  deleted by canonical-identifiers Phase 5.

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

### 3. Projections are now plain `StreamProjection` implementations

The steady-state materialization model is now centered on two durable read
models with a clear plane boundary:

- `SessionIndex` is the agent-plane projection. It owns durable session state
  and is the right home for ACP-keyed prompt-request, permission, and tool-call
  read models keyed by canonical ACP identifiers such as
  `(SessionId, RequestId)` and `(SessionId, ToolCallId)`.
- `HostIndex` is the infrastructure-plane projection. It owns
  `runtime_spec`, `runtime_instance`, and `runtime_endpoints`, which are
  Fireline infrastructure records rather than ACP agent records.

This keeps the projection contract compatible with the canonical-identifiers
split:

- agent-plane projections deserialize ACP-schema identity fields directly
- infrastructure-plane projections keep Fireline host/runtime identity local to
  infra records
- no steady-state projection depends on `prompt_turn_id` or any other synthetic
  lineage key

`ActiveTurnIndex` still implements `StreamProjection` in the current codebase,
but only as transitional runtime-local waiter coordination. It is not part of
the steady-state projection model and is deleted by canonical-identifiers
Phase 5.

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
- `active_turn_index.rs`: transitional local projection logic + waiters

The new structure is:

- `projection.rs`: shared protocol types + shared `StreamProjection` trait
- `state_materializer.rs`: transport and fanout only
- `session_index.rs`: agent-plane session projection
- `host_index.rs`: infrastructure-plane host projection
- `active_turn_index.rs`: transitional compatibility projection plus waiters,
  scheduled for deletion by canonical-identifiers Phase 5

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
