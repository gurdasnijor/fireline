# Observation

## Current API: `fireline.db(...)`

`@fireline/client` exposes `fireline.db()` as the unified entry point for
durable-stream observation. It wraps `createFirelineDB` from
`@fireline/state`, preloads the underlying stream, and hoists collections
onto the DB object so callers can write `db.sessions` or `db.permissions`
directly.

```ts
import fireline from '@fireline/client'

const db = await fireline.db({ stateStreamUrl: agent.state.url })

db.sessions.subscribe((rows) => {
  console.log(rows.map((row) => row.sessionId))
})
```

If `stateStreamUrl` is omitted, `db()` reads `FIRELINE_STREAM_URL` from
the environment and falls back to
`http://localhost:7474/streams/state/default`.

The named export is also available:

```ts
import { db } from '@fireline/client'

const fireDb = await db({ stateStreamUrl: agent.state.url })
```

See:

- [packages/client/src/db.ts](../../packages/client/src/db.ts)
- [packages/state/src/collection.ts](../../packages/state/src/collection.ts)
- [packages/state/src/schema.ts](../../packages/state/src/schema.ts)

### Using `@fireline/state` directly

`createFirelineDB` from `@fireline/state` is still exported and still
works. Use it when you want explicit control over preloading or when you
don't have `@fireline/client` in scope:

```ts
import { createFirelineDB } from '@fireline/state'

const db = createFirelineDB({ stateStreamUrl: agent.state.url })
await db.preload()

db.collections.sessions.subscribe((rows) => {
  console.log(rows.map((row) => row.sessionId))
})
```

The difference:

- `fireline.db(...)` — preloads automatically, hoists collections onto
  the DB object (`db.sessions`, `db.permissions`, …), returns a `Promise`.
- `createFirelineDB(...)` — returns the DB synchronously; you call
  `await db.preload()` yourself and access collections through
  `db.collections.*`.

Both return the same underlying TanStack DB instance.

## Collections

The `FirelineDB` exposes these live collections (via `db.<name>` or
`db.collections.<name>`):

- `sessions`
- `promptTurns`
- `permissions`
- `chunks`
- `childSessionEdges`
- `connections`
- `pendingRequests`
- `terminals`
- `runtimeInstances`

See [packages/state/src/schema.ts](../../packages/state/src/schema.ts)
for the row shapes.

## Subscribe, do not poll

Each collection has a `.subscribe(...)` helper in
[packages/state/src/collection.ts](../../packages/state/src/collection.ts):

```ts
db.permissions.subscribe((rows) => {
  const pending = rows.filter((row) => row.state === 'pending')
  console.log(pending.map((row) => row.requestId))
})
```

This is a thin wrapper over TanStack DB's `subscribeChanges(...)`.

## React pattern: `useLiveQuery`

For React, use `useLiveQuery` from `@tanstack/react-db`:

```tsx
import { eq } from '@tanstack/db'
import { useLiveQuery } from '@tanstack/react-db'

const sessions = useLiveQuery(
  (q) => q.from({ s: db.sessions }),
  [db],
)

const pendingPermissions = useLiveQuery(
  (q) =>
    q
      .from({ p: db.permissions })
      .where(({ p }) => eq(p.state, 'pending')),
  [db],
)
```

The repo's
[examples/live-monitoring/index.ts](../../examples/live-monitoring/index.ts)
is the clean reference for this pattern.

## ACP hooks in React

Observation and ACP are separate planes:

- observation: `fireline.db(...)`
- session: `useAcpClient(...)` from `use-acp`

For React/browser UIs, pair them. For Node, use `agent.connect()` or
`connectAcp(agent.acp)` (see [compose-and-start.md](./compose-and-start.md)).

## Prebuilt query builders

`@fireline/state` exports seven prebuilt live-query helpers from
[packages/state/src/collections](../../packages/state/src/collections):

- `createPendingPermissionsCollection` — pending approvals only
- `createSessionTurnsCollection` — all prompt turns for one ACP session,
  ordered by `startedAt`
- `createTurnChunksCollection` — all chunks for one prompt turn, ordered
  by `seq`
- `createSessionPermissionsCollection` — full approval history for one
  session
- `createActiveTurnsCollection` — prompt turns whose state is `active`
- `createQueuedTurnsCollection` — prompt turns whose state is `queued`,
  ordered by queue position
- `createConnectionTurnsCollection` — prompt turns for one logical
  connection across sessions

```ts
import {
  createPendingPermissionsCollection,
  createSessionTurnsCollection,
} from '@fireline/state'

const pending = createPendingPermissionsCollection({
  permissions: db.permissions,
})

const sessionTurns = createSessionTurnsCollection({
  promptTurns: db.promptTurns,
  sessionId,
})
```

## Why observation matters

The durable stream is the source of truth, so observation is more than
UI rendering:

- approvals wait on stream events
- session history is replayable after restart
- dashboards and bots can subscribe without reaching into sandbox memory

That architectural choice is why Fireline can replace polling-heavy, ad
hoc backend infrastructure with a stream-backed read model.
