# Flamecast-Style Agent Platform Client

> A minimal agent platform client built on Fireline's `compose(sandbox, middleware, agent)` API. Shows how a product like [Flamecast](https://github.com/smithery-ai/flamecast) would use Fireline as its infrastructure layer.

## What this demonstrates

1. **Sandbox provisioning** — `compose().start()` provisions an agent sandbox with middleware and resources
2. **Reactive state observation** — `@fireline/state` + TanStack DB live queries power the UI without polling
3. **ACP session management** — `@agentclientprotocol/sdk` opens sessions inside provisioned sandboxes
4. **Approval flow** — middleware pipeline includes `approve()`, and the UI subscribes to pending permissions via the durable stream
5. **Multi-agent topology** — `peer()` wires two agents for cross-agent collaboration

## Architecture

```
┌─────────────────────────────────────────────────────┐
│  Flamecast Client (React)                            │
│                                                      │
│  ┌──────────────┐  ┌──────────────┐  ┌────────────┐ │
│  │ Agent Panel   │  │ Chat Panel   │  │ State      │ │
│  │              │  │              │  │ Explorer   │ │
│  │ compose()    │  │ ACP SDK      │  │ @fireline/  │ │
│  │ .start()     │  │ prompt()     │  │ state      │ │
│  └──────┬───────┘  └──────┬───────┘  └──────┬─────┘ │
│         │                 │                 │        │
└─────────┼─────────────────┼─────────────────┼────────┘
          │ POST /v1/sandboxes  │ ws://handle.acp    │ SSE handle.state
          ▼                 ▼                 ▼
┌─────────────────────────────────────────────────────┐
│  Fireline Server                                     │
│  ProviderDispatcher → LocalSubprocessProvider        │
│  DurableStreamTracer → durable-streams               │
└─────────────────────────────────────────────────────┘
```

## The code

```typescript
// ============================================================
// Fireline — composition + provisioning
// ============================================================
import { compose, agent, sandbox, middleware, peer } from '@fireline/client'
import { trace, approve, budget, inject } from '@fireline/client/middleware'
import { localPath, streamBlob } from '@fireline/client/resources'

// ============================================================
// Fireline — reactive state observation
// ============================================================
import { createFirelineDB } from '@fireline/state'

// ============================================================
// ACP (third-party — NOT Fireline)
// ============================================================
import { ClientSideConnection, PROTOCOL_VERSION } from '@agentclientprotocol/sdk'

// ============================================================
// React (app framework — NOT Fireline)
// ============================================================
import { useLiveQuery } from '@tanstack/react-db'
import { eq } from '@tanstack/db'
import { useState, useMemo, useEffect } from 'react'

// -----------------------------------------------------------
// 1. Define agents as composable values
// -----------------------------------------------------------

const codeReviewer = compose(
  sandbox({
    resources: [localPath('~/projects/frontend', '/workspace', true)],
    envVars: { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY! },
    labels: { role: 'reviewer', team: 'frontend' },
  }),
  middleware([
    trace(),                                    // log every ACP effect
    inject([{ kind: 'workspace_file', path: '/workspace/README.md' }]),
    approve({ scope: 'tool_calls', timeoutMs: 120_000 }),
    budget({ tokens: 1_000_000 }),
  ]),
  agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
).as('reviewer')

const slackNotifier = compose(
  sandbox({
    envVars: { SLACK_WEBHOOK_URL: process.env.SLACK_WEBHOOK_URL! },
    labels: { role: 'notifier' },
  }),
  middleware([trace()]),
  agent(['node', 'agents/slack-notifier.js']),
).as('notifier')

// -----------------------------------------------------------
// 2. Wire them into a topology
// -----------------------------------------------------------

// peer() connects agents — the reviewer can call the notifier
// via ACP peer calls. The stream carries cross-agent lineage.
const topology = peer(codeReviewer, slackNotifier)

// -----------------------------------------------------------
// 3. Provision and observe
// -----------------------------------------------------------

async function launchReviewSession(serverUrl: string) {
  // One call provisions both sandboxes and wires peer edges
  const handles = await topology.start({ serverUrl })

  // Open ACP session on the reviewer
  const ws = new WebSocket(handles.reviewer.acp.url)
  const conn = new ClientSideConnection(
    createHandler(),
    createWebSocketStream(ws),
  )
  await conn.initialize({
    protocolVersion: PROTOCOL_VERSION,
    clientInfo: { name: 'flamecast-client', version: '0.0.1' },
    clientCapabilities: { fs: { readTextFile: false } },
  })
  const { sessionId } = await conn.newSession({ cwd: '/workspace' })

  // Fire the review prompt — don't await (observation is via the stream)
  conn.prompt({
    sessionId,
    prompt: [{ type: 'text', text: 'Review this codebase for security issues.' }],
  })

  // Subscribe to the durable stream for reactive UI
  const db = createFirelineDB({ stateStreamUrl: handles.reviewer.state.url })
  await db.preload()

  return { handles, sessionId, db, conn }
}

// -----------------------------------------------------------
// 4. React components powered by @fireline/state
// -----------------------------------------------------------

function ReviewDashboard({ db, sessionId }: { db: FirelineDB; sessionId: string }) {
  // Live queries — update as the stream advances. No fetch. No polling.
  const turns = useLiveQuery(q =>
    q.from({ t: db.collections.promptTurns })
      .where(({ t }) => eq(t.sessionId, sessionId))
  )
  const chunks = useLiveQuery(q =>
    q.from({ c: db.collections.chunks })
  )
  const permissions = useLiveQuery(q =>
    q.from({ p: db.collections.permissions })
      .where(({ p }) => eq(p.sessionId, sessionId))
      .where(({ p }) => eq(p.state, 'pending'))
  )
  const edges = useLiveQuery(q =>
    q.from({ e: db.collections.childSessionEdges })
  )

  return (
    <div className="grid grid-cols-3 gap-4">
      {/* Turns — the conversation history */}
      <div>
        <h2>Review Progress</h2>
        {turns.map(turn => (
          <TurnCard key={turn.promptTurnId} turn={turn} />
        ))}
      </div>

      {/* Live streaming chunks */}
      <div>
        <h2>Live Output</h2>
        <ChunkStream chunks={chunks} />
      </div>

      {/* Pending approvals */}
      <div>
        <h2>Pending Approvals</h2>
        {permissions.map(perm => (
          <ApprovalCard
            key={perm.requestId}
            permission={perm}
            onApprove={() => appendApprovalResolved({
              streamUrl: db.stateStreamUrl,
              sessionId: perm.sessionId,
              requestId: perm.requestId,
              allow: true,
            })}
            onDeny={() => appendApprovalResolved({
              streamUrl: db.stateStreamUrl,
              sessionId: perm.sessionId,
              requestId: perm.requestId,
              allow: false,
            })}
          />
        ))}

        {/* Cross-agent lineage */}
        <h2>Agent Collaboration</h2>
        {edges.map(edge => (
          <EdgeCard key={edge.edgeId} edge={edge} />
        ))}
      </div>
    </div>
  )
}
```

## How this maps to Flamecast

| Flamecast concern | Fireline primitive | Implementation |
|---|---|---|
| Agent provisioning | `compose(sandbox, middleware, agent).start()` | One POST to `/v1/sandboxes` provisions the sandbox with middleware + resources |
| Session management | ACP SDK (`ClientSideConnection`) | WebSocket to `handle.acp.url` — standard ACP protocol |
| Live state observation | `@fireline/state` + TanStack DB | `useLiveQuery` over durable-stream subscription — zero polling |
| Permission brokering | `approve()` middleware + stream subscription | Middleware suspends on tool calls; UI subscribes to `permissions` collection |
| Multi-agent orchestration | `peer()` topology operator | Type-safe multi-agent wiring with cross-agent lineage tracking |
| xterm.js terminal | `sandbox.execute(handle, cmd)` | Provider-level exec bypasses ACP for direct shell access |

## Coming soon

- **Cross-host provisioning** — `peer(agent.at('server-a'), agent.at('server-b'))` for multi-cloud topologies ([`docs/proposals/cross-host-discovery.md`](../../docs/proposals/cross-host-discovery.md))
- **Resource discovery** — `streamBlob('resources:tenant-prod', 'codebase')` for cross-host resource mounting ([`docs/proposals/resource-discovery.md`](../../docs/proposals/resource-discovery.md))
- **Secrets injection** — `middleware([secretsProxy({ OPENAI_KEY: { allow: 'api.openai.com' } })])` for credential isolation ([`docs/proposals/deployment-and-remote-handoff.md`](../../docs/proposals/deployment-and-remote-handoff.md) §5)
- **Stream-FS** — live read/write filesystem backed by the durable stream ([`docs/proposals/stream-fs-spike.md`](../../docs/proposals/stream-fs-spike.md))
