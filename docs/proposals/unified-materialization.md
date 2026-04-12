# Unified Materialization for Durable-Streams Projections

## §1. The Current State

Fireline currently has four files participating in durable-streams materialization:

- `crates/fireline-session/src/state_materializer.rs`
- `crates/fireline-session/src/session_index.rs`
- `crates/fireline-session/src/host_index.rs`
- `crates/fireline-session/src/active_turn_index.rs`

The first important clarification is that these are not four independent stream readers anymore. `state_materializer.rs` already owns the single subscribe/replay/live-tail loop. The other three files are projection implementations layered on top of it.

That means the current duplication is subtler than "four separate subscribers":

- `StateMaterializer` owns transport concerns: connect, replay from beginning, live-tail, `preload()`, JSON-array chunk parsing, control-message classification, and fanout to projections.
- `SessionIndex`, `HostIndex`, and `ActiveTurnIndex` each re-implement their own event routing, operation matching, row deserialization, delete semantics, and reset behavior.
- `ActiveTurnIndex` also mixes two concerns: projected state and waiter coordination.
- `trace.rs` and `state_projector.rs` define their own local `StateEnvelope` shapes on the write side instead of sharing the read-side protocol model.

So the real problem is not that Fireline has no abstraction. It already has one. The problem is that the abstraction stops halfway:

- transport lifecycle is unified
- projected state shape is not
- protocol envelope types are not shared
- waiter/read-notification behavior is embedded inside a specific projection
- write-side and read-side protocol modeling can drift independently

There is also some naming drift:

- `StateMaterializer` is really a projection runtime, not just a decoder
- `StateProjection` is async and state-owning, which makes each projection carry its own locking strategy
- the "index" types are really materialized read models

Today this yields a design that works, but forces every new projection to copy the same pattern:

1. Define internal `HashMap` state behind `Arc<RwLock<_>>`.
2. Match on `entity_type`.
3. Match again on `headers.operation`.
4. Deserialize rows ad hoc.
5. Implement `reset()` by clearing local maps.
6. Add any projection-specific signaling out of band.

That is why the code feels fragmented even though the subscriber loop is already centralized.

## §2. The Abstraction

The next step should not be "add another generic materializer." It should be to formalize the one we already have around a typed projection trait:

```rust
pub trait StreamProjection<S>: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn initial_state(&self) -> S;
    fn project(&self, state: &mut S, event: &StateEvent) -> anyhow::Result<()>;

    fn reset(&self, state: &mut S) -> anyhow::Result<()> {
        *state = self.initial_state();
        Ok(())
    }
}
```

With supporting shared protocol types:

```rust
pub enum StateEvent {
    Change(StateChange),
    Control(StateControl),
}

pub struct StateChange {
    pub entity_type: String,
    pub key: String,
    pub value: Option<serde_json::Value>,
    pub old_value: Option<serde_json::Value>,
    pub headers: ChangeHeaders,
}

pub struct ChangeHeaders {
    pub operation: Operation,
    pub txid: Option<String>,
    pub timestamp: Option<String>,
}

pub struct StateControl {
    pub control: ControlKind,
    pub offset: Option<String>,
}
```

And a shared runtime:

```rust
pub struct ProjectionRuntime {
    projections: Vec<Arc<dyn ErasedProjection>>,
}
```

The key design choice is that `project()` should mutate `&mut S` synchronously. The current `StateProjection` trait is async because each index owns its own locks. That pushes synchronization down into every projection and scatters lifecycle concerns across the codebase. A unified materialization layer should invert that:

- the runtime owns stream transport and replay lifecycle
- the runtime owns synchronization around projected state
- the projection is just a pure-ish event-to-state mutator

In practice the reusable building block becomes:

```rust
pub struct MaterializedProjection<P, S> {
    projection: P,
    state: Arc<tokio::sync::RwLock<S>>,
}
```

Each concrete read model then becomes a typed wrapper:

- `SessionIndex = MaterializedProjection<SessionProjection, SessionState>`
- `HostIndex = MaterializedProjection<HostProjection, HostState>`
- `ActiveTurnIndex = MaterializedProjection<ActiveTurnProjection, ActiveTurnState>`

This preserves the current one-reader fanout topology while removing the repeated envelope handling logic.

### One `connect()` + `preload()` pattern

The current `StateMaterializerTask::preload()` behavior is the correct nucleus. Keep that pattern, but make it the public runtime contract:

- `connect(url)` starts one durable-streams reader for one stream
- `preload()` waits until the runtime reaches the live edge
- `snapshot()` reads the current projected state
- `abort()` stops the reader task

That gives every projection the same lifecycle semantics without every projection needing its own transport wrapper.

### Where `ActiveTurnIndex` waiters belong

`ActiveTurnIndex` is the one projection that is not just "state plus query methods." It also contains waiter coordination. That should move out of the core projection state.

The clean split is:

- `ActiveTurnProjection` owns only `ActiveTurnState`
- `ActiveTurnLookup` or `ActiveTurnWaiters` layers notifications on top of state updates

Waiters are not durable state. They are runtime-local read helpers. Treating them as part of the projection itself is what makes `ActiveTurnIndex` look structurally different from the others.

## §3. Alignment with `STATE-PROTOCOL`

Fireline is broadly aligned with the upstream state protocol, but only with the minimal subset.

What is correct today:

- `state_materializer.rs` expects `application/json` chunks to arrive as JSON arrays. That matches `PROTOCOL.md` section 7.1, which requires JSON-mode GET responses to return a JSON array of messages.
- change events use the correct core shape: `type`, `key`, `headers.operation`, `value`
- control events use the correct core shape: `headers.control`, optionally `headers.offset`
- `trace.rs`/`state_projector.rs` emit ordinary change messages rather than inventing a Fireline-specific wrapper

Where Fireline drifts:

1. The read model drops optional protocol fields.

`RawStateEnvelope` and `RawStateHeaders` only model:

- `type`
- `key`
- `value`
- `headers.operation`

The upstream protocol also allows:

- `old_value`
- `headers.txid`
- `headers.timestamp`

Dropping those fields does not make current events invalid, but it means Fireline cannot preserve or exploit protocol features that the spec explicitly reserves.

2. `reset` is only half implemented.

The protocol says a `reset` control message tells clients to clear materialized state and restart from the indicated offset. Fireline clears projections, but it does not restart from `headers.offset`. It continues with the existing live reader. That is functional drift, not just unused metadata.

3. Snapshot controls are treated as comments.

`snapshot-start` and `snapshot-end` are logged and ignored. That is not a protocol violation, but it means `preload()` has no notion of "consistent snapshot boundary." It only knows "reader reached `up_to_date` once."

4. Fireline has no shared protocol type across write and read paths.

The same envelope shape is modeled independently in:

- `fireline-session/src/state_materializer.rs`
- `fireline-harness/src/trace.rs`
- `fireline-harness/src/state_projector.rs`

That is the biggest protocol-maintenance risk in the current design.

### Recommendation

Introduce one shared `StateEvent` model in `fireline-session` and make both reader-side and writer-side code depend on it. `trace.rs` should emit that type. `state_materializer.rs` should parse that type. The protocol should exist once in code.

## §4. Alignment with durable-streams `client-rust`

Fireline is using the Rust client correctly in the narrow sense, but not completely in the operational sense.

### What is already good

- `StateMaterializer` uses the official client rather than hand-rolling HTTP/SSE.
- It reads from `Offset::Beginning`, which is correct for rebuilding in-memory state from the authoritative stream.
- It uses live tailing after replay, which matches the intended catch-up then tail model.
- The writer side uses `Producer.append_json()` and `flush()` for the lifecycle-sensitive helper writes.

### What is missing or underused

1. No checkpoint/resume strategy.

The client API is built around saving `chunk.next_offset` and resuming from it later. Fireline always starts from `Offset::Beginning`. That is acceptable for small streams and runtime-local caches, but it is still the simplest possible usage of the client.

2. No explicit reconnection policy above the iterator.

The upstream iterator already handles a useful amount of transport behavior:

- SSE fallback to long-poll when SSE is unsupported
- cursor tracking
- SSE reconnect by re-establishing the stream on the next call

Fireline benefits from that, but its outer loop still treats retryable errors as "immediately call `next_chunk()` again" with no backoff, no rebuild, and no escalation path for repeated transient failure. That is thin error handling, not a full materialization runtime policy.

3. `LiveMode::Sse` is narrower than the documented happy path.

The client README shows `LiveMode::Auto` as the normal consumer surface. Fireline hard-codes `LiveMode::Sse`. The iterator does have fallback behavior, so this is not catastrophic, but `Auto` better expresses intent: "prefer SSE, but use the supported live mode."

4. No client configuration surface.

`Client::builder()` supports:

- default headers
- dynamic header providers
- timeout configuration
- retry configuration

`StateMaterializer` uses `Client::new()` directly. That means auth headers, request tuning, and future transport policy all live outside the abstraction.

5. Retry handling is incomplete for protocol-level recovery.

`OffsetGone`, auth failures, and repeated 5xx/429 conditions are not surfaced as materializer states with recovery policy. They are just loop outcomes. A real projection runtime should expose states like:

- replaying
- live
- degraded
- reset-required
- failed

### Recommendation

Keep using the official client, but wrap it more deliberately:

- use `Client::builder()` and inject headers/timeouts explicitly
- prefer `LiveMode::Auto`
- persist optional checkpoints when replay cost becomes non-trivial
- add bounded backoff and health state around retryable failures
- treat `reset`/`OffsetGone` as first-class recovery paths, not generic errors

## §5. `trace.rs` Instrumentation

`crates/fireline-harness/src/trace.rs` is mostly emitting valid `STATE-PROTOCOL` change messages.

That is true for:

- `emit_host_spec_persisted`
- `emit_host_endpoints_persisted`
- the `StateProjector`-derived changes forwarded by `DurableStreamTracer::write_event`

The common emitted shape is:

```json
{
  "type": "...",
  "key": "...",
  "headers": { "operation": "insert|update|delete" },
  "value": { ... }
}
```

That is valid protocol shape for change events.

The divergences are these:

1. The implementation only targets the minimal subset.

It never emits:

- `old_value`
- `headers.txid`
- `headers.timestamp`

That is acceptable today, but it means the code should not claim to model the full protocol.

2. The protocol model is duplicated.

`trace.rs` has its own `StateEnvelope<T>`. `state_projector.rs` has another. `state_materializer.rs` has a read-side `RawStateEnvelope`. This is the main source of future drift.

3. Write durability semantics are uneven.

`emit_host_instance_started`, `emit_host_instance_stopped`, `emit_host_spec_persisted`, and `emit_host_endpoints_persisted` flush explicitly. The ordinary `write_event()` path appends without flushing each event. That is a reasonable batching choice, but it means "format correctness" and "durable visibility timing" are not the same property.

4. Fireline uses semantic operation labels inconsistently.

`runtime_spec` is always emitted as `insert`, and `runtime_endpoints` is always emitted as `update`. Those choices may be operationally harmless, but they are domain semantics layered on the protocol, not protocol requirements. The unified abstraction should make those policy choices explicit.

### Recommendation

Move all change-envelope construction behind a shared helper, for example:

```rust
state::change("runtime_endpoints", key, Operation::Update, value)
state::control(ControlKind::Reset, Some(offset))
```

Then `trace.rs` stops being a second protocol definition.

## §6. Migration Plan

The migration should be incremental and keep the existing one-reader topology intact.

### Phase 1: Normalize protocol types

Create one shared state-protocol module in `fireline-session`:

- `StateEvent`
- `StateChange`
- `StateControl`
- `Operation`
- `ControlKind`

Update:

- `state_materializer.rs` to parse that model
- `trace.rs` to emit that model
- `state_projector.rs` to construct that model

This is the highest-leverage cleanup because it removes write/read drift.

### Phase 2: Replace `StateProjection` with typed `StreamProjection<S>`

Introduce:

- `StreamProjection<S>`
- `MaterializedProjection<P, S>`
- `ProjectionRuntime`

Keep `StateMaterializerTask::preload()` behavior, but rename the surrounding types to reflect what they really are.

### Phase 3: Port the current indices

Port each read model one by one:

1. `SessionIndex`
2. `HostIndex`
3. `ActiveTurnIndex`

Each port should:

- move deserialization into `project(&mut S, event)`
- move shared reset behavior to `initial_state()`
- remove per-projection `Arc<RwLock<_>>` ownership from the projection logic

### Phase 4: Pull waiter logic out of projected state

Split `ActiveTurnIndex` into:

- `ActiveTurnProjection`
- `ActiveTurnNotifier` or `ActiveTurnLookup`

That keeps the projection abstraction clean and prevents future indices from embedding runtime-local signaling into the materialized state layer.

### Phase 5: Add runtime health and recovery policy

Extend the projection runtime with explicit lifecycle states:

- replaying
- live
- degraded
- failed

Handle:

- retryable transport errors with bounded backoff
- `reset` by restarting from the indicated offset
- `OffsetGone` as a forced rebuild case

### Phase 6: Delete the ad hoc names

After the typed abstraction is in place:

- rename `StateMaterializer` to `ProjectionRuntime` or `StreamMaterializer`
- rename `StateProjection` to `StreamProjection`
- keep `SessionIndex`/`HostIndex` as public read-model names, but make them wrappers over the shared runtime

## Recommendation

Fireline should not build a brand-new materializer system. It should finish the one it already started.

The correct end state is:

- one shared durable-streams reader per stream
- one shared protocol model used by readers and writers
- one typed projection trait for event-to-state mutation
- one shared `connect()` + `preload()` lifecycle
- thin, typed read models on top

That preserves the good part of the current architecture, namely the single replay/live-tail loop, while removing the duplicated projection boilerplate and the write/read protocol drift that will otherwise keep recurring.

## References

- Durable Streams base protocol: <https://github.com/durable-streams/durable-streams/blob/main/PROTOCOL.md>
- Durable Streams state protocol: <https://github.com/durable-streams/durable-streams/blob/main/packages/state/STATE-PROTOCOL.md>
- Durable Streams Rust client: <https://github.com/durable-streams/durable-streams/tree/main/packages/client-rust>
