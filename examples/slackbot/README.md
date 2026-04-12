# Slackbot — Stream-Driven Agent Integration

> A Slack bot that provisions Fireline agents on mention, observes progress via `@fireline/state` stream subscription, and handles approval flows — all without long-lived WebSocket connections.

## What this demonstrates

1. **Event-driven provisioning** — Slack mention → `compose().start()` → fire-and-forget prompt
2. **Stream-based observation** — `@fireline/state` subscription for completion and permissions (not WebSocket polling)
3. **Stateless service** — the bot holds no session state; the durable stream IS the state
4. **Approval brokering** — pending permissions surface in Slack; user replies resolve them via the stream

## Architecture

```
Slack                     Slackbot Service              Fireline
  │                           │                            │
  │ @mention "review PR #42"  │                            │
  ├──────────────────────────▶│                            │
  │                           │ compose().start()          │
  │                           ├───────────────────────────▶│ provisions sandbox
  │                           │ conn.prompt() (fire&forget)│
  │                           ├───────────────────────────▶│ agent starts working
  │                           │                            │
  │                           │ createFirelineDB()         │
  │                           │ ← stream subscription ────┤ durable stream events
  │                           │                            │
  │ "Agent working on it..." ◀┤ (on session created)       │
  │                           │                            │
  │ "Agent needs approval..." ◀┤ (on pending permission)   │
  │                           │                            │
  │ User replies "approve"    │                            │
  ├──────────────────────────▶│ appendApprovalResolved()   │
  │                           ├───────────────────────────▶│ stream append
  │                           │                            │ agent continues
  │                           │                            │
  │ "Review complete: ..."   ◀┤ (on turn completed)        │
```

## The code

```typescript
// ============================================================
// Fireline — composition
// ============================================================
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, approve } from '@fireline/client/middleware'
import { gitRepo } from '@fireline/client/resources'

// ============================================================
// Fireline — state observation
// ============================================================
import { createFirelineDB } from '@fireline/state'

// ============================================================
// Fireline — stream write helpers
// ============================================================
import { appendApprovalResolved } from '@fireline/client/events'

// ============================================================
// ACP (third-party — NOT Fireline)
// ============================================================
import { ClientSideConnection, PROTOCOL_VERSION } from '@agentclientprotocol/sdk'

// ============================================================
// Slack (third-party — NOT Fireline)
// ============================================================
import { App } from '@slack/bolt'

// ============================================================
// App code — NOT Fireline
// ============================================================
const FIRELINE_URL = process.env.FIRELINE_URL ?? 'http://localhost:4440'

// Track thread → session mappings (the bot's only state)
const threadSessions = new Map<string, {
  sessionId: string
  stateStreamUrl: string
  sandboxId: string
}>()

const app = new App({
  token: process.env.SLACK_BOT_TOKEN!,
  signingSecret: process.env.SLACK_SIGNING_SECRET!,
})

// -----------------------------------------------------------
// 1. On mention — provision + fire prompt + subscribe
// -----------------------------------------------------------

app.event('app_mention', async ({ event, say }) => {
  const threadTs = event.thread_ts ?? event.ts

  // Provision a sandbox with the compose API
  const handle = await compose(
    sandbox({
      envVars: { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY! },
      labels: { slack_thread: threadTs, slack_channel: event.channel },
    }),
    middleware([
      trace(),
      approve({ scope: 'tool_calls', timeoutMs: 300_000 }),
    ]),
    agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
  ).start({ serverUrl: FIRELINE_URL, name: `slack-${threadTs}` })

  // Open ACP session
  const ws = new WebSocket(handle.acp.url)
  const conn = new ClientSideConnection(
    { onPermission: () => {} },  // handled via stream
    createWebSocketStream(ws),
  )
  await conn.initialize({
    protocolVersion: PROTOCOL_VERSION,
    clientInfo: { name: 'fireline-slackbot', version: '0.0.1' },
    clientCapabilities: { fs: { readTextFile: false } },
  })
  const { sessionId } = await conn.newSession({ cwd: '/' })

  // Store the mapping
  threadSessions.set(threadTs, {
    sessionId,
    stateStreamUrl: handle.state.url,
    sandboxId: handle.id,
  })

  // Fire the prompt — DON'T AWAIT
  conn.prompt({
    sessionId,
    prompt: [{ type: 'text', text: event.text.replace(/<@[^>]+>/, '').trim() }],
  })

  await say({ text: '🔥 Agent is working on it...', thread_ts: threadTs })

  // -----------------------------------------------------------
  // 2. Subscribe to the stream for completion + permissions
  // -----------------------------------------------------------

  const db = createFirelineDB({ stateStreamUrl: handle.state.url })
  await db.preload()

  // Watch for completion
  db.collections.promptTurns.subscribe((turns) => {
    const completed = turns.find(
      t => t.sessionId === sessionId && t.completedAt != null
    )
    if (completed) {
      // Find the agent's response in the chunks
      const chunks = db.collections.chunks
      // Post the result to Slack
      say({
        text: `✅ Agent finished (${completed.stopReason})`,
        thread_ts: threadTs,
      })
    }
  })

  // Watch for approval requests
  db.collections.permissions.subscribe((perms) => {
    const pending = perms.find(
      p => p.sessionId === sessionId && p.state === 'pending'
    )
    if (pending) {
      say({
        text: `⚠️ Agent needs approval: *${pending.title ?? 'tool call'}*\n\nReply \`approve\` or \`deny\` in this thread.`,
        thread_ts: threadTs,
      })
    }
  })
})

// -----------------------------------------------------------
// 3. Handle approval responses in thread
// -----------------------------------------------------------

app.message(async ({ message, say }) => {
  if (message.subtype || !('thread_ts' in message) || !message.thread_ts) return

  const session = threadSessions.get(message.thread_ts)
  if (!session) return

  const text = ('text' in message ? message.text : '')?.toLowerCase().trim()
  if (text !== 'approve' && text !== 'deny') return

  const allow = text === 'approve'

  // Read the pending permission from the stream
  const db = createFirelineDB({ stateStreamUrl: session.stateStreamUrl })
  await db.preload()
  const pending = (await db.collections.permissions.getAll()).find(
    p => p.sessionId === session.sessionId && p.state === 'pending'
  )

  if (!pending) {
    await say({ text: 'No pending approval to respond to.', thread_ts: message.thread_ts })
    return
  }

  // Resolve the approval via the durable stream
  await appendApprovalResolved({
    streamUrl: session.stateStreamUrl,
    sessionId: session.sessionId,
    requestId: pending.requestId,
    allow,
  })

  await say({
    text: allow ? '✅ Approved — agent continuing.' : '❌ Denied — agent stopping.',
    thread_ts: message.thread_ts,
  })
})

// -----------------------------------------------------------
// Start
// -----------------------------------------------------------

;(async () => {
  await app.start(process.env.PORT ?? 3000)
  console.log('⚡ Fireline Slackbot running')
})()
```

## Why this is better than the WebSocket pattern

The [current Flamecast Slackbot guide](https://flamecast.mintlify.app/guides/slackbot) uses a persistent WebSocket connection to observe agent progress. That works but requires:

- A long-lived process (not serverless-compatible)
- WebSocket reconnection logic
- In-memory session tracking across reconnects

The Fireline pattern uses **durable-stream subscription** instead:

- **Stateless** — the bot holds one `Map<threadTs, sessionInfo>` for routing, but all real state is on the stream
- **Serverless-compatible** — each Slack event handler provisions, fires, and subscribes independently
- **Crash-resilient** — if the bot restarts, it re-subscribes to the same stream URLs from the `threadSessions` map (which can be persisted to Redis/KV if needed)
- **No WebSocket management** — `createFirelineDB` handles the SSE subscription and reconnection internally

## Key patterns

1. **Fire-and-forget prompt:** `conn.prompt(...)` without `await` — the response is observed via the stream, not the prompt return value
2. **Stream as the observation API:** `db.collections.promptTurns.subscribe(...)` and `db.collections.permissions.subscribe(...)` — zero polling, zero custom WebSocket
3. **Approval via stream append:** `appendApprovalResolved({ streamUrl, sessionId, requestId, allow })` — the approval response is a durable-stream append, not an API call to the sandbox
4. **Thread-scoped sessions:** `labels: { slack_thread: threadTs }` — the sandbox is labeled with the Slack thread so it can be found later if needed

## Coming soon

- **Webhook delivery** — instead of stream subscription in the bot, Fireline pushes completion/permission events to a callback URL ([Flamecast webhooks RFC](https://flamecast.mintlify.app/rfcs/webhooks))
- **Cross-host provisioning** — provision the agent sandbox on a cloud server while the bot runs locally ([`docs/proposals/cross-host-discovery.md`](../../docs/proposals/cross-host-discovery.md))
- **Resource mounting** — attach repo code to the sandbox so the agent can review PRs ([`docs/proposals/resource-discovery.md`](../../docs/proposals/resource-discovery.md))
