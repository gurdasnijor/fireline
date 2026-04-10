# Session Load and Reconnect

## Purpose

`session/load` is the protocol-side reattachment primitive that lets a client
come back to an existing logical ACP session.

In Fireline, it should pair with durable state-stream replay.

## What `session/load` solves

It solves protocol reattachment:

- a client reconnects
- it loads an existing session by `sessionId`
- the runtime replays enough session history for the client to resume context

## What it does not solve alone

It does not, by itself, define:

- multiplayer/shared-session semantics
- who is controller vs observer
- how permission prompts are coordinated across multiple attached clients

Those are higher-layer concerns.

## Fireline model

The intended reconnect path is:

1. client reconnects to the runtime's ACP endpoint
2. client sends `session/load(sessionId)`
3. runtime or terminal agent replays session updates
4. client combines replayed ACP updates with durable state-stream replay as needed

## Capability negotiation

Not every underlying agent will support `loadSession` equally.

So Fireline should be explicit about:

- whether the downstream terminal agent advertises `loadSession`
- whether Fireline can offer a runtime-managed fallback
- what degraded mode means when neither exists

## Relationship to state-stream replay

State-stream replay and `session/load` solve different layers.

- `session/load` is the protocol-side reattachment primitive
- state-stream replay is the durable observation primitive

Fireline should use both, not choose one over the other.

## Relationship to mesh peering

Mesh peering can land before `session/load`, but long-running distributed work
will eventually want reconnect semantics.

That is why `session/load` is a follow-on foundation, not optional long-term.

## Proposed solution

Fireline should treat `session/load` as a coordination problem, not as a
replacement for the ACP SDK's session machinery.

The design split is:

- ACP SDK owns live protocol sessions:
  - `initialize`
  - `session/new`
  - `session/load`
  - `session/prompt`
  - `session/update`
- Fireline owns durable logical session records:
  - where a session lives
  - whether it is resumable
  - how it relates to parent prompt turns and peer sessions
  - what runtime/provider should be restarted to reattach

### Core model

The system should model one logical distributed session graph as many
runtime-local ACP sessions.

- one entry runtime receives the original client session
- peer calls may create additional child ACP sessions on remote runtimes
- Fireline persists the relationships between those sessions
- a reconnect attaches to the specific session the client wants, while the
  durable state stream remains the source of truth for observing the wider graph

### Materialized session index

Add durable `session` rows to Fireline's state stream and materialize an
in-memory index keyed by `sessionId`.

The minimum record shape should be:

```ts
type SessionRecord = {
  sessionId: string
  runtimeKey: string
  runtimeId: string
  nodeId: string
  logicalConnectionId?: string
  status: 'starting' | 'active' | 'idle' | 'broken' | 'closed'
  supportsLoadSession: boolean
  traceId?: string
  parentPromptTurnId?: string
  parentSessionId?: string
  createdAtMs: number
  updatedAtMs: number
  lastSeenAtMs: number
}
```

This record is Fireline-owned durable state. It is not a new ACP protocol
object, and it should not be duplicated into a second local file store.

### What gets persisted

At minimum, Fireline should persist in the state stream:

- the runtime binding:
  - `runtimeKey`
  - `runtimeId`
  - `nodeId`
- the session identity:
  - `sessionId`
- resumability:
  - `supportsLoadSession`
  - current `status`
- lineage:
  - `traceId`
  - `parentPromptTurnId`
  - optional `parentSessionId`

For peered work, this is the crucial addition: Fireline must also persist the
child-session binding created by a peer call.

That means when runtime A prompts runtime B:

- parent prompt turn on A is known
- child session on B is known
- Fireline stores that edge durably

Without that binding, observers can reconstruct history, but reconnect logic
cannot reliably reattach to the correct remote child session.

## Runtime ownership model

The current hosted `/acp` path creates a fresh conductor and terminal subprocess
per WebSocket connection. That is sufficient for the baseline, but not for
restart-safe `session/load`.

To support durable reattachment:

- runtime ownership must move above the transient WebSocket connection
- a runtime must keep or recreate session backends independently of one client
  transport attachment
- `/acp` connections should attach to existing runtime/session state rather than
  implicitly defining it

This does not require replacing the SDK's session engine.

It does require Fireline to become explicit about the lifetime of:

- runtime process
- terminal agent instance
- ACP session binding
- client attachment

## Load flow

The intended happy path is:

1. client obtains `RuntimeDescriptor`
2. client connects to `runtime.acpUrl`
3. client sends `initialize`
4. client sends `session/load(sessionId)`
5. Fireline looks up `SessionRecord` in its materialized `SessionIndex`
6. Fireline ensures the target runtime is present
7. Fireline checks whether the downstream terminal supports `loadSession`
8. if supported, Fireline delegates to the SDK/agent path
9. Fireline resumes streaming `session/update`
10. client combines live ACP replay with durable state-stream replay if needed

### Restart path

After a runtime or host restart:

1. `RuntimeHost` recreates or restarts the runtime from `runtimeKey`
2. Fireline replays durable `session` rows for that runtime into its local
   `SessionIndex`
3. each session is marked:
   - resumable, or
   - known-but-nonresumable
4. client reconnects and requests `session/load`
5. Fireline either:
   - reattaches through the downstream `loadSession` capability, or
   - returns a degraded-mode failure while preserving durable history

## What Fireline should not do

Fireline should not:

- reimplement `ActiveSession`
- invent a Fireline-only prompt/update loop
- define a parallel session wire protocol
- treat state-stream replay as a replacement for protocol reattachment

The ACP SDK already owns live session semantics. Fireline should coordinate,
persist, and restore around that.

## State projection implications

The state stream should remain the canonical durable observation surface.

For this work, Fireline should project at least:

- session records
- child session edges
- resumability flags
- last-seen timestamps

That can be done either as:

- new `session` / `session_edge` state rows, or
- an extension of existing runtime/prompt-turn rows

The key invariant is:

- replaying durable state must be sufficient to reconstruct which sessions
  existed, where they lived, and how they were related

## Recommended execution slice

The next implementation slice should be:

### `07-durable-session-index-and-load-local`

Scope:

- add durable `session` rows
- materialize a `SessionIndex` from replay + live state updates
- capture child session bindings from peer calls
- decouple session identity from WebSocket lifetime
- implement local `session/load` coordination against one real agent/runtime
- explicitly defer remote restart recovery when the downstream agent does not
  support `loadSession`

Acceptance criteria:

- client can disconnect and reconnect to the same local session
- `session/load` reattaches to the same logical Fireline session
- session records survive a Fireline restart through state-stream replay
- peer-created child sessions are durably discoverable from Fireline state
- no custom ACP session engine is introduced

## Parallel work with low overlap

The main `session/load` implementation will touch runtime/session ownership
surfaces, so it should stay focused.

Good parallel tracks with low overlap are:

- TS observation ergonomics:
  - better `@fireline/state` helpers for querying sessions and prompt turns
  - no overlap with Rust session coordination
- TS peer primitives:
  - `client.peer.list()` and `client.peer.call(...)`
  - mostly package/client and docs work
- provider expansion:
  - add a second provider stub or remote-backed provider to `RuntimeHost`
  - low overlap if the runtime descriptor stays stable

Higher-overlap work that should not run in parallel with the core session-load
slice:

- restructuring hosted runtime/session ownership
- changing runtime descriptor identity semantics
- changing lineage fields on prompt turns
