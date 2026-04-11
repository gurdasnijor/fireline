# 07: Durable Session Index and Load Coordination

## Objective

Prove that Fireline can durably catalog sessions and coordinate `session/load`
without inventing a custom session engine or pretending cross-transport resume
already exists.

This slice is about coordination and persistence:

- Fireline persists session rows to the durable state stream
- Fireline materializes a local session index from replay + live updates
- Fireline can restart and still know what session a client is asking for
- Fireline returns an explicit non-resumable error when the downstream terminal
  does not support reattach
- the ACP SDK remains the live session implementation

## What this slice proves

- `sessionId` is durably indexed by Fireline
- `session/load` is coordinated against the materialized index
- Fireline can distinguish resumable vs nonresumable sessions
- Fireline can restart and still return the durable session record for a known
  session
- non-resumable sessions fail explicitly instead of silently disappearing

## Scope

- add durable `session` rows
- build a materialized `SessionIndex` from the state stream
- coordinate `session/load` against the materialized durable record
- return explicit `session_not_resumable` when `supportsLoadSession` is false
- keep state-stream replay and ACP reattachment as separate layers

## Non-goals

- shared-session / multiplayer semantics
- custom prompt/update loops
- a Fireline-only session protocol
- full cross-transport session resume
- runtime-owned terminal/session lifetime
- full remote crash recovery when the downstream agent lacks `loadSession`

## Core design rules

1. The ACP SDK owns live session semantics.
2. Fireline owns durable logical session records.
3. `session/load` is coordinated by Fireline, not reimplemented by Fireline.
4. Durable state must be sufficient to discover session topology after restart.
5. If the successor cannot resume, Fireline must fail explicitly and include
   the durable record in `error.data._meta.fireline`.

## Acceptance criteria

- creating a session writes a durable `session` row
- `session/load(sessionId)` consults the materialized `SessionIndex`
- unsupported reattach returns `session_not_resumable`
- the error payload includes the durable session record in
  `error.data._meta.fireline.sessionRecord`
- restarting Fireline does not lose session lookup because the index replays
  from the durable state stream
- no new custom ACP session engine is introduced

## Validation

- Rust integration test:
  - create session
  - call `session/load`
  - assert `session_not_resumable`
  - assert durable `SessionRecord` is present in `error.data._meta.fireline`
- restart integration test:
  - stop and restart Fireline
  - assert session index rebuilds from durable state
  - assert `session/load` still returns the same durable session record

## Storage note

The restart-replay proof requires persistent durable-stream storage.

- default in-memory stream storage is sufficient for live catalog lookup
- restart replay requires `file-durable` or `acid` stream storage
- this slice does not claim restart durability when the runtime is backed only
  by in-memory stream storage

## Deferred to Slice 08

- runtime-owned terminal/session lifetime
- actual cross-transport resume semantics
- long-lived session backends independent of transient ACP attachments
