// Fireline
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { appendApprovalResolved } from '@fireline/client/events'
import { approve, trace } from '@fireline/client/middleware'
import { createFirelineDB } from '@fireline/state'

// Third-party
import { App } from '@slack/bolt'

// App code
import { openNodeAcpConnection } from '../shared/acp-node.js'

const bot = new App({ token: process.env.SLACK_BOT_TOKEN!, signingSecret: process.env.SLACK_SIGNING_SECRET! })
const threads = new Map<string, { sessionId: string; stateStreamUrl: string }>()

bot.event('app_mention', async ({ event, say }) => {
  const threadTs = event.thread_ts ?? event.ts
  const handle = await compose(
    sandbox({ labels: { slackChannel: event.channel, slackThread: threadTs } }),
    middleware([trace(), approve({ scope: 'tool_calls', timeoutMs: 300_000 })]),
    agent((process.env.AGENT_COMMAND ?? 'npx -y @anthropic-ai/claude-code-acp').split(' ')),
  ).start({ serverUrl: process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440', name: `slack-${threadTs}` })
  const acp = await openNodeAcpConnection(handle.acp.url, 'fireline-slackbot')
  const { sessionId } = await acp.connection.newSession({ cwd: '/', mcpServers: [] })
  threads.set(threadTs, { sessionId, stateStreamUrl: handle.state.url })
  void acp.connection.prompt({ sessionId, prompt: [{ type: 'text', text: event.text.replace(/<@[^>]+>/g, '').trim() }] }).catch(console.error)
  setTimeout(() => void acp.close().catch(console.error), 1_000)
  await say({ text: 'Agent is working on it.', thread_ts: threadTs })
  void observeThread(threadTs, sessionId, handle.state.url, say)
})

bot.message(async ({ message, say }) => {
  const thread = message.thread_ts ? threads.get(message.thread_ts) : undefined
  const decision = message.text?.trim().toLowerCase()
  if (message.subtype || !message.thread_ts || !thread || (decision !== 'approve' && decision !== 'deny')) return
  const db = createFirelineDB({ stateStreamUrl: thread.stateStreamUrl }); await db.preload()
  const pending = db.collections.permissions.toArray.find((entry) => entry.sessionId === thread.sessionId && entry.state === 'pending')
  db.close()
  if (!pending) return void (await say({ text: 'No pending approval for this thread.', thread_ts: message.thread_ts }))
  await appendApprovalResolved({
    streamUrl: thread.stateStreamUrl,
    sessionId: thread.sessionId,
    requestId: pending.requestId,
    allow: decision === 'approve',
    resolvedBy: 'slackbot',
  })
  await say({ text: decision === 'approve' ? 'Approved. Agent continuing.' : 'Denied. Agent stopping.', thread_ts: message.thread_ts })
})

async function observeThread(threadTs: string, sessionId: string, stateStreamUrl: string, say: (message: { text: string; thread_ts?: string }) => Promise<unknown>) {
  const db = createFirelineDB({ stateStreamUrl }); await db.preload()
  const seenTurns = new Set<string>(); const seenPermissions = new Set<string>()
  const publish = async () => {
    for (const entry of db.collections.permissions.toArray.filter((row) => row.sessionId === sessionId && row.state === 'pending' && !seenPermissions.has(row.requestId))) {
      seenPermissions.add(entry.requestId)
      await say({ text: `Approval needed: ${entry.title ?? 'tool call'}. Reply with approve or deny.`, thread_ts: threadTs })
    }
    for (const turn of db.collections.promptTurns.toArray.filter((row) => row.sessionId === sessionId && row.completedAt && !seenTurns.has(row.promptTurnId))) {
      seenTurns.add(turn.promptTurnId)
      const text = db.collections.chunks.toArray.filter((chunk) => chunk.promptTurnId === turn.promptTurnId).map((chunk) => chunk.content).join('').trim() || turn.stopReason || 'Completed.'
      await say({ text: `Review complete:\n${text.slice(0, 1200)}`, thread_ts: threadTs })
    }
  }
  db.collections.permissions.subscribe(() => { void publish() })
  db.collections.promptTurns.subscribe(() => { void publish() })
}

void bot.start(process.env.PORT ?? 3000).then(() => console.log('fireline slackbot ready'))
