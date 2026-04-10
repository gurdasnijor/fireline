# 07: Durable Session Index and Load Local

## Objective

Prove that Fireline can durably reattach to an existing local ACP session
without inventing a custom session engine.

This slice is about coordination and persistence:

- Fireline persists session rows to the durable state stream
- Fireline materializes a local session index from replay + live updates
- Fireline can restart and still know what session a client is asking for
- the ACP SDK remains the live session implementation

## What this slice proves

- `sessionId` is durably indexed by Fireline
- a disconnected client can reconnect and call `session/load`
- the same logical session is reattached after a Fireline restart
- peer-created child sessions are durably discoverable
- Fireline can distinguish resumable vs nonresumable sessions

## Scope

- add durable `session` rows
- build a materialized `SessionIndex` from the state stream
- record child session bindings from peer calls
- coordinate `session/load` against one real downstream agent/runtime
- keep state-stream replay and ACP reattachment as separate layers

## Non-goals

- shared-session / multiplayer semantics
- custom prompt/update loops
- a Fireline-only session protocol
- full remote crash recovery when the downstream agent lacks `loadSession`

## Core design rules

1. The ACP SDK owns live session semantics.
2. Fireline owns durable logical session records.
3. `session/load` is coordinated by Fireline, not reimplemented by Fireline.
4. Durable state must be sufficient to discover session topology after restart.

## Acceptance criteria

- creating a session writes a durable `session` row
- local reconnect via `session/load(sessionId)` succeeds
- restarting Fireline does not lose session lookup because the index replays
  from the durable state stream
- peer-created child sessions are durably bound to parent prompt turns
- no new custom ACP session engine is introduced

## Validation

- Rust integration test:
  - create session
  - disconnect
  - reconnect and `session/load`
  - assert same logical session resumes
- restart integration test:
  - stop and restart Fireline
  - assert session index rebuilds from durable state
- mesh integration test:
  - create child session through peer call
  - assert durable child-session binding exists
