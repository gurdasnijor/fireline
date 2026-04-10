# `_meta.fireline` Contract

## Purpose

Fireline uses ACP `_meta` for runtime-specific protocol extensions that need to
travel with live ACP messages.

This document defines the currently supported `fireline` metadata fields and
error payloads.

## Current uses

Fireline currently uses `_meta.fireline` for:

- peer-call lineage on `initialize` / `_proxy/initialize`
- explicit `session/load` coordination errors

Anything durable or queryable must still be projected into the Fireline state
stream. `_meta.fireline` is the live protocol extension channel, not the
durable contract.

## Lineage fields

Peer calls stamp lineage into top-level ACP `_meta`:

```json
{
  "_meta": {
    "fireline": {
      "traceId": "trace:123",
      "parentPromptTurnId": "turn:abc"
    }
  }
}
```

Current supported fields:

- `traceId`
- `parentPromptTurnId`

Current deferred fields:

- `callerNodeId`
- `callerRuntimeId`

Fireline readers currently accept both:

- nested `_meta.fireline.traceId` / `_meta.fireline.parentPromptTurnId`
- legacy flat keys:
  - `_meta["fireline/trace-id"]`
  - `_meta["fireline/parent-prompt-turn-id"]`

Writers should prefer the nested `fireline` object form.

## `session/load` error payloads

Fireline attaches durable lookup metadata under `error.data._meta.fireline`.

### `session_not_found`

Returned when the requested session is not present in the materialized
`SessionIndex`.

Shape:

```json
{
  "_meta": {
    "fireline": {
      "error": "session_not_found",
      "sessionId": "session:123"
    }
  }
}
```

### `session_not_resumable`

Returned when Fireline knows the session durably, but the downstream terminal
does not advertise `loadSession`.

Shape:

```json
{
  "_meta": {
    "fireline": {
      "error": "session_not_resumable",
      "reason": "downstream_load_session_unsupported",
      "sessionRecord": {
        "sessionId": "session:123",
        "runtimeKey": "runtime:abc",
        "runtimeId": "fireline:test:xyz",
        "nodeId": "node:test",
        "logicalConnectionId": "conn:1",
        "state": "active",
        "supportsLoadSession": false,
        "createdAt": 0,
        "updatedAt": 0,
        "lastSeenAt": 0
      }
    }
  }
}
```

The error code itself is a Fireline-specific server error. The durable record
is included so clients and control planes can make an explicit decision without
guessing at host-local state.
