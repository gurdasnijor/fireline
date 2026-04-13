# `@fireline/state`

When an agent is doing real work, logs are too late and raw stream events are
too low-level. You need a typed read model that updates as the durable stream
advances.

`@fireline/state` is that package. It materializes the Fireline state stream
into live collections you can query, filter, and project in application code.

## Two Entry Points

If you are already using `@fireline/client`, the ergonomic entry point is:

```ts
import fireline from '@fireline/client'

const db = await fireline.db({ stateStreamUrl: handle.state.url })
```

That wrapper calls this package under the hood, preloads the DB for you, and
hoists the collections onto the returned object.

If you want the raw state package directly, use `createFirelineDB(...)`:

```ts
import { createFirelineDB } from '@fireline/state'

const db = createFirelineDB({
  stateStreamUrl: 'http://127.0.0.1:7474/v1/stream/fireline-state-demo',
})

await db.preload()
```

## Fastest Path

```ts
import { createFirelineDB, extractChunkTextPreview } from '@fireline/state'

const db = createFirelineDB({
  stateStreamUrl: 'http://127.0.0.1:7474/v1/stream/fireline-state-demo',
})

await db.preload()

const subscription = db.collections.chunks.subscribe((rows) => {
  const text = rows.map((row) => extractChunkTextPreview(row.update)).join('')
  console.log(text)
})
```

The package story is simple:

- materialize the durable stream once
- read the base collections
- derive narrower views where you need them
- subscribe to changes instead of polling

## Base Constructor

### `createFirelineDB(config)`

Builds the DB synchronously from a durable stream URL.

```ts
import { createFirelineDB } from '@fireline/state'

const db = createFirelineDB({
  stateStreamUrl: 'http://127.0.0.1:7474/v1/stream/fireline-state-demo',
  headers: { authorization: 'Bearer demo-token' },
})
```

`FirelineDBConfig`:

```ts
interface FirelineDBConfig {
  stateStreamUrl: string
  headers?: Record<string, string>
  signal?: AbortSignal
}
```

### `db.preload()`

Reads the current stream state and materializes the first snapshot.

```ts
await db.preload()
```

Unlike `fireline.db()` from `@fireline/client`, `createFirelineDB(...)` does
not preload automatically.

### `db.close()`

Stops the internal stream subscriptions and closes the underlying DB.

```ts
db.close()
```

## Base Collections

The shipped base collection names are:

- `promptRequests`
- `permissions`
- `sessions`
- `chunks`

If you were looking for older names like `promptTurns` or `turnChunks`, those
are view-level concepts now. The base package surface uses `promptRequests` and
`chunks`, then gives you derived helpers below.

### `db.collections.promptRequests`

All prompt requests across all sessions.

```ts
const completed = db.collections.promptRequests.toArray.filter(
  (row) => row.state === 'completed',
)
```

Each row is a `PromptRequestRow` with fields such as:

- `sessionId`
- `requestId`
- `text`
- `state`
- `position`
- `startedAt`
- `completedAt`
- `stopReason`

### `db.collections.permissions`

Projected approval state across all sessions.

```ts
const pending = db.collections.permissions.toArray.filter(
  (row) => row.state === 'pending',
)
```

This collection collapses `permission_request` plus `approval_resolved` events
into one row per request.

### `db.collections.sessions`

Session lifecycle rows keyed by ACP `sessionId`.

```ts
const resumable = db.collections.sessions.toArray.filter(
  (row) => row.supportsLoadSession,
)
```

### `db.collections.chunks`

Canonical request chunks for all sessions.

```ts
import { isToolCallSessionUpdate } from '@fireline/state'

const toolUpdates = db.collections.chunks.toArray.filter((row) =>
  isToolCallSessionUpdate(row.update),
)
```

Each `ChunkRow` carries the raw `SessionUpdate` in `row.update`.

## Reactive Subscribe Surface

### `collection.subscribe(callback)`

The base collections expose a small convenience wrapper:

```ts
const subscription = db.collections.permissions.subscribe((rows) => {
  console.log(rows.map((row) => `${row.requestId}:${row.state}`))
})

subscription.unsubscribe()
```

Behavior:

- the callback runs immediately with the current `toArray` snapshot
- it runs again after every collection change
- `unsubscribe()` stops the updates

This helper is added to the four base collections returned by
`createFirelineDB(...)`.

## Derived Live Views

These helpers create narrower TanStack live-query collections from the base
collections.

### `createPendingPermissionsCollection({ permissions })`

Reactive view over pending approvals only.

```ts
import { createPendingPermissionsCollection } from '@fireline/state'

const pending = createPendingPermissionsCollection({
  permissions: db.collections.permissions,
})
```

### `createSessionPromptRequestsCollection({ promptRequests, sessionId })`

Reactive view over one session's prompt requests, ordered by `startedAt`.

```ts
import { createSessionPromptRequestsCollection } from '@fireline/state'

const sessionRequests = createSessionPromptRequestsCollection({
  promptRequests: db.collections.promptRequests,
  sessionId,
})
```

This is the current package answer to "show me the turns for one session."

### `createRequestChunksCollection({ chunks, requestId, sessionId? })`

Reactive view over one request's chunks, ordered by `createdAt`.

```ts
import { createRequestChunksCollection } from '@fireline/state'

const requestChunks = createRequestChunksCollection({
  chunks: db.collections.chunks,
  sessionId,
  requestId,
})
```

This is the current package answer to "show me the chunk stream for one turn."

### `createSessionPermissionsCollection({ permissions, sessionId })`

Reactive view over one session's permission history.

```ts
import { createSessionPermissionsCollection } from '@fireline/state'

const sessionPermissions = createSessionPermissionsCollection({
  permissions: db.collections.permissions,
  sessionId,
})
```

### `createActiveTurnsCollection({ promptRequests })`

Reactive view over prompt requests whose state is currently `active`.

```ts
import { createActiveTurnsCollection } from '@fireline/state'

const active = createActiveTurnsCollection({
  promptRequests: db.collections.promptRequests,
})
```

### `createQueuedTurnsCollection({ promptRequests })`

Reactive view over queued prompt requests, ordered by `position`.

```ts
import { createQueuedTurnsCollection } from '@fireline/state'

const queued = createQueuedTurnsCollection({
  promptRequests: db.collections.promptRequests,
})
```

## Session Update Helpers

Chunk rows carry raw ACP `SessionUpdate` payloads. These helpers give you the
common "show me something human-readable" path without hard-coding update-shape
switches everywhere.

### `extractChunkTextPreview(update)`

Returns a compact text preview for message chunks and tool-call updates.

```ts
import { extractChunkTextPreview } from '@fireline/state'

const text = requestChunks.toArray
  .map((row) => extractChunkTextPreview(row.update))
  .join('')
```

### `isToolCallSessionUpdate(update)`

Returns `true` for `tool_call` and `tool_call_update` events.

```ts
import { isToolCallSessionUpdate } from '@fireline/state'

const toolRows = db.collections.chunks.toArray.filter((row) =>
  isToolCallSessionUpdate(row.update),
)
```

### `sessionUpdateKind(update)`

Returns the raw `sessionUpdate` kind string.

```ts
import { sessionUpdateKind } from '@fireline/state'

const kind = sessionUpdateKind(row.update)
```

### `sessionUpdateStatus(update)`

Returns the `status` field when the update carries one.

```ts
import { sessionUpdateStatus } from '@fireline/state'

const status = sessionUpdateStatus(row.update)
```

### `sessionUpdateTitle(update)`

Returns `title` when present, otherwise `toolName` when present.

```ts
import { sessionUpdateTitle } from '@fireline/state'

const label = sessionUpdateTitle(row.update)
```

### `sessionUpdateToolCallId(update)`

Returns the tool call id when the update carries one.

```ts
import { sessionUpdateToolCallId } from '@fireline/state'

const toolCallId = sessionUpdateToolCallId(row.update)
```

## Key Helpers

### `requestIdCollectionKey(requestId)`

Normalizes ACP request ids into a stable collection key fragment.

```ts
import { requestIdCollectionKey } from '@fireline/state'

const key = requestIdCollectionKey(requestId)
```

### `promptRequestCollectionKey(sessionId, requestId)`

Builds the stable per-request collection key.

```ts
import { promptRequestCollectionKey } from '@fireline/state'

const key = promptRequestCollectionKey(sessionId, requestId)
```

## Types And Schema

The package exports the row and ACP-facing types most consumers need:

- `FirelineDB`
- `FirelineDBConfig`
- `FirelineCollections`
- `PromptRequestRow`
- `PermissionRow`
- `SessionRow`
- `ChunkRow`
- `RuntimeInstanceRow`
- `SessionId`
- `RequestId`
- `ToolCallId`
- `SessionUpdate`
- `StopReason`
- `StateEvent`

Example:

```ts
import type { ChunkRow, FirelineDB, RequestId, SessionId } from '@fireline/state'
```

### `firelineState`

Exports the Fireline state schema for consumers that need the schema object
itself.

```ts
import { firelineState } from '@fireline/state'
```

## Related Docs

- [Observation](../../docs/guide/observation.md)
- [API: State](../../docs/guide/api/state.md)
- [Concepts: Observation Model](../../docs/guide/concepts/observation-model.md)
- [`@fireline/client`](../client/README.md)
