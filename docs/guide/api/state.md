# `@fireline/state`

Reference for the current Fireline observation package. This page stays on the
API surface: constructors, collections, reactive helpers, and exported types.
For the observation model and why the durable stream is the source of truth,
use [Observation](../observation.md) and `Concepts C8`.

## `fireline.db(options?)`

`fireline.db()` lives in `@fireline/client`, but it is the common entry point
for the `@fireline/state` read model.

```ts
function db(options?: FirelineDbOptions): Promise<FirelineDB>

interface FirelineDbOptions {
  stateStreamUrl?: string
  headers?: Record<string, string>
}
```

Use it when you want the state DB preloaded and its collections hoisted onto
the returned object.

```ts
import fireline from '@fireline/client'

const db = await fireline.db({ stateStreamUrl: handle.state.url })

db.sessions.subscribe((rows) => {
  console.log(rows.map((row) => row.sessionId))
})
```

Notes:

- If `stateStreamUrl` is omitted, the client wrapper reads
  `FIRELINE_STREAM_URL` and then falls back to
  `http://localhost:7474/streams/state/default`.
- The returned object is the same underlying DB as `createFirelineDB(...)`,
  with `promptRequests`, `permissions`, `sessions`, and `chunks` attached as
  top-level properties.

## `createFirelineDB(config)`

```ts
function createFirelineDB(config: FirelineDBConfig): FirelineDB

interface FirelineDBConfig {
  stateStreamUrl: string
  headers?: Record<string, string>
  signal?: AbortSignal
}
```

Use `createFirelineDB(...)` when you want direct access to the state package
without the `@fireline/client` wrapper.

```ts
import { createFirelineDB } from '@fireline/state'

const db = createFirelineDB({
  stateStreamUrl: 'http://127.0.0.1:7474/streams/state/demo',
})

await db.preload()
console.log(db.collections.sessions.toArray.length)
```

The returned DB currently exposes:

- `collections`: the Fireline materialized collections
- `preload(): Promise<void>`: reads the stream and materializes current rows
- `close(): void`: unsubscribes the internal synchronizers and closes the stream
- `stream`: the underlying durable-stream transport handle
- `utils`: helper utilities returned by `createStreamDB(...)`

## `FirelineDB.collections`

```ts
interface FirelineCollections {
  promptRequests: ObservableCollection<PromptRequestRow>
  permissions: ObservableCollection<PermissionRow>
  sessions: ObservableCollection<SessionRow>
  chunks: ObservableCollection<ChunkRow>
}
```

These are the current collection names exported by `@fireline/state`.
Turn-shaped state lives in `promptRequests`. Chunk and tool-call updates live
in `chunks`; there is no separate top-level `toolCalls` collection today.

### `collections.promptRequests`

```ts
const promptRequests: ObservableCollection<PromptRequestRow>
```

Prompt requests for all sessions, keyed by session plus request id.

```ts
const queued = db.collections.promptRequests.toArray.filter(
  (row) => row.state === 'queued',
)
```

### `collections.permissions`

```ts
const permissions: ObservableCollection<PermissionRow>
```

Projected permission state for all sessions. The projection collapses
`permission_request` and `approval_resolved` events into one row per request.

```ts
const pending = db.collections.permissions.toArray.filter(
  (row) => row.state === 'pending',
)
```

### `collections.sessions`

```ts
const sessions: ObservableCollection<SessionRow>
```

Session lifecycle rows keyed by `sessionId`.

```ts
const activeSessions = db.collections.sessions.toArray.filter(
  (row) => row.state === 'active',
)
```

### `collections.chunks`

```ts
const chunks: ObservableCollection<ChunkRow>
```

Canonical request chunks for all sessions. Each row carries the raw
`SessionUpdate` payload in `row.update`.

```ts
import { isToolCallSessionUpdate } from '@fireline/state'

const toolCallChunks = db.collections.chunks.toArray.filter((row) =>
  isToolCallSessionUpdate(row.update),
)
```

## `ObservableCollection<T>.subscribe(callback)`

```ts
type ObservableCollection<T extends object> = Collection<T, string> & {
  subscribe(callback: (rows: T[]) => void): { unsubscribe(): void }
}
```

`subscribe(...)` is a small convenience wrapper over TanStack DB
`subscribeChanges(...)`. It immediately calls `callback` with the current
`toArray` contents, then calls it again after every change.

```ts
const subscription = db.collections.permissions.subscribe((rows) => {
  console.log(rows.map((row) => `${row.requestId}:${row.state}`))
})

subscription.unsubscribe()
```

## Live Query Helpers

These helpers return TanStack live-query collections derived from the base
collections above.

### `createPendingPermissionsCollection(opts)`

```ts
function createPendingPermissionsCollection(opts: {
  permissions: Collection<PermissionRow, string>
}): Collection<PermissionRow, string>
```

Reactive view of pending permissions only.

```ts
import { createPendingPermissionsCollection } from '@fireline/state'

const pending = createPendingPermissionsCollection({
  permissions: db.collections.permissions,
})
```

### `createSessionPromptRequestsCollection(opts)`

```ts
function createSessionPromptRequestsCollection(
  opts: SessionPromptRequestsOptions,
): Collection<PromptRequestRow, string>

interface SessionPromptRequestsOptions {
  promptRequests: Collection<PromptRequestRow, string>
  sessionId: SessionId
}
```

Reactive view of all prompt requests for one session, ordered by `startedAt`
ascending.

```ts
import { createSessionPromptRequestsCollection } from '@fireline/state'

const sessionRequests = createSessionPromptRequestsCollection({
  promptRequests: db.collections.promptRequests,
  sessionId,
})
```

### `createRequestChunksCollection(opts)`

```ts
function createRequestChunksCollection(
  opts: RequestChunksOptions,
): Collection<ChunkRow, string>

interface RequestChunksOptions {
  chunks: Collection<ChunkRow, string>
  sessionId?: SessionId
  requestId: RequestId
}
```

Reactive view of all chunks for one request, ordered by `createdAt` ascending.
If you already know the session, pass `sessionId` to narrow the collection.

```ts
import { createRequestChunksCollection } from '@fireline/state'

const requestChunks = createRequestChunksCollection({
  chunks: db.collections.chunks,
  sessionId,
  requestId,
})
```

### `createSessionPermissionsCollection(opts)`

```ts
function createSessionPermissionsCollection(
  opts: SessionPermissionsOptions,
): Collection<PermissionRow, string>

interface SessionPermissionsOptions {
  permissions: Collection<PermissionRow, string>
  sessionId: SessionId
}
```

Reactive view of all permission rows for one session, ordered by `createdAt`
ascending.

```ts
import { createSessionPermissionsCollection } from '@fireline/state'

const sessionPermissions = createSessionPermissionsCollection({
  permissions: db.collections.permissions,
  sessionId,
})
```

### `createActiveTurnsCollection(opts)`

```ts
function createActiveTurnsCollection(opts: {
  promptRequests: Collection<PromptRequestRow, string>
}): Collection<PromptRequestRow, string>
```

Reactive view of prompt requests whose state is `active`.

```ts
import { createActiveTurnsCollection } from '@fireline/state'

const active = createActiveTurnsCollection({
  promptRequests: db.collections.promptRequests,
})
```

### `createQueuedTurnsCollection(opts)`

```ts
function createQueuedTurnsCollection(opts: {
  promptRequests: Collection<PromptRequestRow, string>
}): Collection<PromptRequestRow, string>
```

Reactive view of prompt requests whose state is `queued`, ordered by
`position` ascending.

```ts
import { createQueuedTurnsCollection } from '@fireline/state'

const queued = createQueuedTurnsCollection({
  promptRequests: db.collections.promptRequests,
})
```

## Session Update Helpers

`ChunkRow.update` stores a canonical ACP `SessionUpdate`. These helpers extract
common values without requiring callers to pattern-match the payload by hand.

### `extractChunkTextPreview(update)`

```ts
function extractChunkTextPreview(update: SessionUpdate): string
```

Returns a short text preview for message chunks and tool-call updates.

```ts
import { extractChunkTextPreview } from '@fireline/state'

const text = db.collections.chunks.toArray
  .map((row) => extractChunkTextPreview(row.update))
  .join('')
```

### `isToolCallSessionUpdate(update)`

```ts
function isToolCallSessionUpdate(update: SessionUpdate): boolean
```

Returns `true` for `tool_call` and `tool_call_update` variants.

```ts
import { isToolCallSessionUpdate } from '@fireline/state'

const toolCalls = db.collections.chunks.toArray.filter((row) =>
  isToolCallSessionUpdate(row.update),
)
```

### `sessionUpdateKind(update)`

```ts
function sessionUpdateKind(update: SessionUpdate): string
```

Returns the `sessionUpdate` discriminator string when present.

```ts
import { sessionUpdateKind } from '@fireline/state'

const kinds = db.collections.chunks.toArray.map((row) =>
  sessionUpdateKind(row.update),
)
```

### `sessionUpdateStatus(update)`

```ts
function sessionUpdateStatus(update: SessionUpdate): string | undefined
```

Returns the `status` field when the update carries one.

```ts
import { sessionUpdateStatus } from '@fireline/state'

const statuses = db.collections.chunks.toArray
  .map((row) => sessionUpdateStatus(row.update))
  .filter(Boolean)
```

### `sessionUpdateTitle(update)`

```ts
function sessionUpdateTitle(update: SessionUpdate): string | undefined
```

Returns `title` when present, otherwise falls back to `toolName`.

```ts
import { sessionUpdateTitle } from '@fireline/state'

const titles = db.collections.chunks.toArray
  .map((row) => sessionUpdateTitle(row.update))
  .filter(Boolean)
```

### `sessionUpdateToolCallId(update)`

```ts
function sessionUpdateToolCallId(update: SessionUpdate): string | undefined
```

Returns the update's tool-call id when present.

```ts
import { sessionUpdateToolCallId } from '@fireline/state'

const toolCallIds = db.collections.chunks.toArray
  .map((row) => sessionUpdateToolCallId(row.update))
  .filter(Boolean)
```

## Type Exports

`@fireline/state` currently exports these types for consumers:

- Stream types: `StateEvent`
- ACP types: `SessionId`, `RequestId`, `ToolCallId`, `SessionUpdate`,
  `StopReason`, `PromptRequestRef`, `ToolInvocationRef`
- DB types: `FirelineDB`, `FirelineDBConfig`, `FirelineCollections`
- Row types: `PromptRequestRow`, `PermissionRow`, `SessionRow`, `ChunkRow`,
  `RuntimeInstanceRow`
- Helper option types: `SessionPromptRequestsOptions`,
  `RequestChunksOptions`, `SessionPermissionsOptions`

```ts
import type {
  ChunkRow,
  FirelineDB,
  PermissionRow,
  PromptRequestRow,
  RequestId,
  SessionId,
} from '@fireline/state'
```

## Schema Export

### `firelineState`

The package also exports `firelineState`, the underlying durable-stream state
schema used by `createFirelineDB(...)`.

```ts
import { firelineState } from '@fireline/state'
```

Use it when you are building directly on `@durable-streams/state` rather than
through the higher-level Fireline DB wrapper.
