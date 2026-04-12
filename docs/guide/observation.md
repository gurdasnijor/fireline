# Observation

## Current API: `createFirelineDB(...)`

The proposal vocabulary sometimes talks about `fireline.db()`. That convenience does **not** exist in the current TypeScript client.

Today, observation lives in `@fireline/state`:

```ts
import { createFirelineDB } from '@fireline/state'

const db = createFirelineDB({ stateStreamUrl: handle.state.url })
await db.preload()
```

See:

- [packages/state/src/collection.ts](../../packages/state/src/collection.ts)
- [packages/state/src/schema.ts](../../packages/state/src/schema.ts)

Under the hood this is built on `createStreamDB(...)` from `@durable-streams/state`.

## Collections

The current `FirelineDB` exposes these live collections:

- `sessions`
- `promptTurns`
- `permissions`
- `chunks`
- `childSessionEdges`
- `connections`
- `pendingRequests`
- `terminals`
- `runtimeInstances`

These are defined in [packages/state/src/schema.ts](../../packages/state/src/schema.ts).

## Subscribe, do not poll

Each collection gets a convenience `.subscribe(...)` helper in [packages/state/src/collection.ts](../../packages/state/src/collection.ts):

```ts
db.collections.permissions.subscribe((rows) => {
  const pending = rows.filter((row) => row.state === 'pending')
  console.log(pending.map((row) => row.requestId))
})
```

That helper is just a thin wrapper over TanStack DB’s `subscribeChanges(...)`, but it makes the “reactive, not polling” usage pattern obvious.

## React pattern: `useLiveQuery`

For React, use `useLiveQuery` from `@tanstack/react-db`:

```tsx
import { eq } from '@tanstack/db'
import { useLiveQuery } from '@tanstack/react-db'

const sessions = useLiveQuery(
  (q) => q.from({ s: db.collections.sessions }),
  [db],
)

const pendingPermissions = useLiveQuery(
  (q) =>
    q
      .from({ p: db.collections.permissions })
      .where(({ p }) => eq(p.state, 'pending')),
  [db],
)
```

The repo’s [examples/live-monitoring/index.ts](../../examples/live-monitoring/index.ts) is the clean reference for this pattern.

## ACP hooks in React

Observation and ACP are separate planes:

- observation: `createFirelineDB(...)`
- session: `useAcpClient(...)`

For React/browser UIs, the intended pairing is:

- `@fireline/state` for durable observation
- `use-acp` for ACP session state and prompts

## Prebuilt query builders

`@fireline/state` already exports seven prebuilt live-query helpers from [packages/state/src/collections](../../packages/state/src/collections):

- `createPendingPermissionsCollection`
  Pending approvals only.
- `createSessionTurnsCollection`
  All prompt turns for one ACP session, ordered by `startedAt`.
- `createTurnChunksCollection`
  All chunks for one prompt turn, ordered by `seq`.
- `createSessionPermissionsCollection`
  Full approval history for one session, not just pending items.
- `createActiveTurnsCollection`
  Prompt turns whose state is `active`.
- `createQueuedTurnsCollection`
  Prompt turns whose state is `queued`, ordered by queue position.
- `createConnectionTurnsCollection`
  Prompt turns for one logical connection across sessions.

Example:

```ts
import {
  createPendingPermissionsCollection,
  createSessionTurnsCollection,
} from '@fireline/state'

const pending = createPendingPermissionsCollection({
  permissions: db.collections.permissions,
})

const sessionTurns = createSessionTurnsCollection({
  promptTurns: db.collections.promptTurns,
  sessionId,
})
```

These helpers are already exported. Most of the example apps currently do not use them enough.

## Why observation matters

The durable stream is the source of truth, so observation is more than UI rendering:

- approvals wait on stream events
- session history is replayable after restart
- dashboards and bots can subscribe without reaching into sandbox memory

That architectural choice is why Fireline can replace polling-heavy, ad hoc backend infrastructure with a stream-backed read model.
