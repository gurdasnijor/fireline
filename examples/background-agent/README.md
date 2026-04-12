# Background Agent — The Ramp Inspect Pattern

> Fire-and-forget agent provisioning with a reactive dashboard. Inspired by [Ramp's Inspect](https://builders.ramp.com/post/why-we-built-our-background-agent) — *"you can go home and let it cook."*

## What this demonstrates

1. **Fire-and-forget provisioning** — `compose().start()` + `conn.prompt()` without `await` on the response
2. **Reactive dashboard** — `@fireline/state` + `useLiveQuery` renders agent progress without polling
3. **Approval flow** — middleware pipeline includes `approve()`, and the dashboard subscribes to pending permissions via the durable stream
4. **Multiple concurrent tasks** — provision N agents in parallel, observe all of them from one stream subscription

## The pattern

```
User submits task                      User opens dashboard
       │                                      │
       ▼                                      ▼
compose(sandbox, middleware, agent)    createFirelineDB({ stateStreamUrl })
       │                                      │
       ▼                                      ▼
harness.start() → SandboxHandle       useLiveQuery(db.collections.promptTurns)
       │                                      │
       ▼                                      ▼
conn.prompt('...')  ← fire & forget   React renders as stream advances
       │                                      │
       ▼                                      ▼
Agent works in background              Dashboard shows live progress
```

**The key insight from Ramp:** *"Because sessions are fast to start and effectively free to run, you can use them without rationing."* With Fireline, provisioning a sandbox is one POST. The agent runs in the background. The user monitors progress through the durable stream — no WebSocket held open, no long-polling, no server-sent-events from the control plane. The stream IS the API.

## The code

### Task submission service

```typescript
// ============================================================
// Fireline — composition
// ============================================================
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, approve, budget } from '@fireline/client/middleware'
import { gitRepo } from '@fireline/client/resources'

// ============================================================
// App code (NOT Fireline)
// ============================================================
import express from 'express'

const app = express()
app.use(express.json())

const FIRELINE_URL = process.env.FIRELINE_URL ?? 'http://localhost:4440'
const DURABLE_STREAMS_URL = process.env.DURABLE_STREAMS_URL ?? 'http://localhost:4437/v1/stream'

app.post('/api/tasks', async (req, res) => {
  const { repoUrl, branch, description, userId } = req.body
  const taskId = `task-${Date.now()}`

  // 1. Compose the agent harness
  const harness = compose(
    sandbox({
      resources: [gitRepo(repoUrl, branch, '/workspace')],
      envVars: { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY! },
      labels: { task: taskId, user: userId, repo: repoUrl },
    }),
    middleware([
      trace(),
      approve({ scope: 'tool_calls', timeoutMs: 300_000 }),  // 5 min approval timeout
      budget({ tokens: 2_000_000 }),
    ]),
    agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
  )

  // 2. Provision the sandbox
  const handle = await harness.start({
    serverUrl: FIRELINE_URL,
    name: taskId,
  })

  // 3. Open ACP and fire the prompt — DON'T AWAIT the response
  const { ClientSideConnection, PROTOCOL_VERSION } = await import('@agentclientprotocol/sdk')
  const ws = new WebSocket(handle.acp.url)
  const conn = new ClientSideConnection(
    { onPermission: () => {} },  // permissions handled via stream subscription in the dashboard
    createWebSocketStream(ws),
  )
  await conn.initialize({
    protocolVersion: PROTOCOL_VERSION,
    clientInfo: { name: 'background-agent-service', version: '0.0.1' },
    clientCapabilities: { fs: { readTextFile: false } },
  })
  const { sessionId } = await conn.newSession({ cwd: '/workspace' })

  // Fire and forget — the agent works in the background
  conn.prompt({
    sessionId,
    prompt: [{ type: 'text', text: description }],
  })

  // 4. Return the handle immediately — user monitors via the dashboard
  res.json({
    taskId,
    sandboxId: handle.id,
    stateStreamUrl: handle.state.url,
    sessionId,
  })
})

app.listen(3000, () => console.log('Task service ready on :3000'))
```

### React dashboard — reactive observation

```typescript
// ============================================================
// Fireline — state observation
// ============================================================
import { createFirelineDB, type FirelineDB } from '@fireline/state'

// ============================================================
// TanStack DB (peer dependency of @fireline/state)
// ============================================================
import { useLiveQuery } from '@tanstack/react-db'
import { eq } from '@tanstack/db'

// ============================================================
// React (app framework — NOT Fireline)
// ============================================================
import { useState, useMemo, useEffect } from 'react'

// ============================================================
// Fireline — stream write helpers (for approval responses)
// ============================================================
import { appendApprovalResolved } from '@fireline/client/events'

function TaskDashboard({ stateStreamUrl, sessionId }: {
  stateStreamUrl: string
  sessionId: string
}) {
  const db = useMemo(
    () => createFirelineDB({ stateStreamUrl }),
    [stateStreamUrl],
  )

  // Preload the stream — replays from beginning, then stays connected
  const [ready, setReady] = useState(false)
  useEffect(() => {
    db.preload().then(() => setReady(true))
    return () => db.close()
  }, [db])

  if (!ready) return <div>Connecting to agent stream...</div>

  return (
    <div className="grid grid-cols-2 gap-6 p-6">
      <ProgressPanel db={db} sessionId={sessionId} />
      <ApprovalPanel db={db} sessionId={sessionId} stateStreamUrl={stateStreamUrl} />
    </div>
  )
}

function ProgressPanel({ db, sessionId }: { db: FirelineDB; sessionId: string }) {
  // Live query — updates as the stream advances. ZERO polling.
  const turns = useLiveQuery(q =>
    q.from({ t: db.collections.promptTurns })
      .where(({ t }) => eq(t.sessionId, sessionId))
  )
  const latestTurn = turns[turns.length - 1]

  const chunks = useLiveQuery(q =>
    q.from({ c: db.collections.chunks })
      .where(({ c }) => eq(c.promptTurnId, latestTurn?.promptTurnId ?? ''))
  )

  return (
    <div>
      <h2 className="text-xl font-bold mb-4">Agent Progress</h2>
      <div className="text-sm text-gray-500 mb-2">
        {turns.length} turns completed
        {latestTurn?.state === 'active' && ' — agent is thinking...'}
        {latestTurn?.state === 'completed' && ` — finished (${latestTurn.stopReason})`}
      </div>

      {/* Live streaming output */}
      <pre className="bg-gray-900 text-green-400 p-4 rounded text-xs overflow-auto max-h-96">
        {chunks.map(c => c.content).join('')}
      </pre>

      {/* Turn history */}
      <div className="mt-4 space-y-2">
        {turns.map(turn => (
          <div key={turn.promptTurnId} className="border rounded p-3">
            <span className="font-mono text-xs">{turn.state}</span>
            {turn.text && <p className="mt-1 text-sm">{turn.text.slice(0, 200)}</p>}
          </div>
        ))}
      </div>
    </div>
  )
}

function ApprovalPanel({ db, sessionId, stateStreamUrl }: {
  db: FirelineDB
  sessionId: string
  stateStreamUrl: string
}) {
  // Live query for pending permissions — reactive, no polling
  const pendingPermissions = useLiveQuery(q =>
    q.from({ p: db.collections.permissions })
      .where(({ p }) => eq(p.sessionId, sessionId))
      .where(({ p }) => eq(p.state, 'pending'))
  )

  const handleApprove = async (requestId: string) => {
    await appendApprovalResolved({
      streamUrl: stateStreamUrl,
      sessionId,
      requestId,
      allow: true,
    })
  }

  const handleDeny = async (requestId: string) => {
    await appendApprovalResolved({
      streamUrl: stateStreamUrl,
      sessionId,
      requestId,
      allow: false,
    })
  }

  return (
    <div>
      <h2 className="text-xl font-bold mb-4">
        Pending Approvals
        {pendingPermissions.length > 0 && (
          <span className="ml-2 bg-yellow-500 text-black rounded-full px-2 py-0.5 text-xs">
            {pendingPermissions.length}
          </span>
        )}
      </h2>

      {pendingPermissions.length === 0 ? (
        <p className="text-gray-500">No pending approvals</p>
      ) : (
        <div className="space-y-3">
          {pendingPermissions.map(perm => (
            <div key={perm.requestId} className="border border-yellow-500 rounded p-4">
              <p className="font-medium">{perm.title ?? 'Tool call requires approval'}</p>
              <div className="mt-3 flex gap-2">
                <button
                  onClick={() => handleApprove(perm.requestId)}
                  className="bg-green-600 text-white px-4 py-1 rounded"
                >
                  Approve
                </button>
                <button
                  onClick={() => handleDeny(perm.requestId)}
                  className="bg-red-600 text-white px-4 py-1 rounded"
                >
                  Deny
                </button>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
```

## Key insight: the stream IS the coordination substrate

The task submission service and the dashboard never talk to each other directly. They both talk to the durable stream:

- **Service** writes: provisions sandbox → agent writes session/turn/chunk/permission events to the stream
- **Dashboard** reads: subscribes to the stream via `createFirelineDB` → TanStack DB materializes the state → React renders

This is the Ramp insight made concrete: *"each session gets its own SQLite database"* — in our case, each session gets its own projection over the shared durable stream. The stream is the universal coordination substrate. No custom WebSocket. No queue. No database. Just the stream.

## Running multiple tasks

```typescript
// Launch 5 review tasks in parallel — each gets its own sandbox
const tasks = await Promise.all(
  repos.map(repo =>
    compose(
      sandbox({ resources: [gitRepo(repo.url, 'main', '/workspace')], labels: { repo: repo.name } }),
      middleware([trace(), approve({ scope: 'tool_calls' })]),
      agent(['claude-code-acp']),
    ).start({ serverUrl: FIRELINE_URL, name: `review-${repo.name}` })
  ),
)

// One stream subscription sees ALL tasks
const db = createFirelineDB({ stateStreamUrl: tasks[0].state.url })
// Every task's sessions, turns, chunks, permissions — all in one reactive view
```

## Coming soon

- **Pooled providers** — pre-warmed sandboxes for sub-100ms provisioning ([`docs/proposals/sandbox-provider-model.md`](../../docs/proposals/sandbox-provider-model.md) §5)
- **Webhook-driven approvals** — POST approval responses to a callback URL instead of writing to the stream directly ([Flamecast webhooks RFC](https://flamecast.mintlify.app/rfcs/webhooks))
- **Resource discovery** — pre-publish repos to the stream so sandboxes mount them instantly ([`docs/proposals/resource-discovery.md`](../../docs/proposals/resource-discovery.md))
