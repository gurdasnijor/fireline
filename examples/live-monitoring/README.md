# Live Monitoring

When people ask for agent observability, they usually mean three concrete questions: which sessions are alive, which runs are blocked on approval, and what the agent is doing right now. Most stacks answer that with polling loops, a dashboard-only API, and a lot of glue code to merge live updates with stored history.

This example shows the Fireline version of that operator wallboard. The browser opens one state stream with `fireline.db(...)`, subscribes to the current `sessions`, `promptRequests`, `permissions`, and `chunks`, and derives the dashboard from those rows. No polling. No second observability service. No ACP control client mixed into the read path.

## What This Example Shows

1. `fireline.db({ stateStreamUrl })` opens the current durable read model.
2. `db.sessions.subscribe(...)`, `db.promptRequests.subscribe(...)`, `db.permissions.subscribe(...)`, and `db.chunks.subscribe(...)` push the latest rows whenever the stream advances.
3. The React view derives counts and per-session cards from those four collections, so the UI always reflects current durable state.

## The Code

```tsx
const db = await fireline.db({ stateStreamUrl })

const subscriptions = [
  db.sessions.subscribe((rows) => {
    current.sessions = rows
    publish()
  }),
  db.promptRequests.subscribe((rows) => {
    current.promptRequests = rows
    publish()
  }),
  db.permissions.subscribe((rows) => {
    current.permissions = rows
    publish()
  }),
  db.chunks.subscribe((rows) => {
    current.chunks = rows
    publish()
  }),
]
```

That is the product message. The durable stream is already the source of truth, so the monitoring surface just subscribes to the same state the runtime uses.

The dashboard then turns those rows into operator facts: active sessions, pending approvals, request backlog, recent tool activity, and per-session summaries built from `extractChunkTextPreview(row.update)` and `isToolCallSessionUpdate(row.update)`.

## Run It

```bash
pnpm --dir .. install --ignore-workspace --lockfile=false
cd examples/live-monitoring
pnpm install
VITE_FIRELINE_STATE_STREAM_URL=http://127.0.0.1:7474/streams/state/demo pnpm start
```

Start any Fireline workflow first, copy its `stateStream` URL, and use that as `VITE_FIRELINE_STATE_STREAM_URL`. Good pairings are:

- [`../background-task`](../background-task) when you want to watch a long-running job.
- [`../code-review-agent`](../code-review-agent) when you want to see approvals and tool activity.
- [`../multi-agent-team`](../multi-agent-team) when you want to watch multiple peers share one outcome.

If you want a custom dashboard, this example is the pattern to copy: subscribe to the current collections, derive the view you need, and let the durable stream stay the only source of truth.
