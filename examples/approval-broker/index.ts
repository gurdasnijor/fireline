// Fireline
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { appendApprovalResolved } from '@fireline/client/events'
import { approve, trace } from '@fireline/client/middleware'
import { createFirelineDB } from '@fireline/state'

// Third-party
import { createServer, type IncomingMessage } from 'node:http'

// User code
import { openNodeAcpConnection } from '../shared/acp-node.js'

const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const webhookUrl = process.env.APPROVAL_WEBHOOK ?? 'http://127.0.0.1:8787/approve'
const webhook = new URL(webhookUrl)
const agentCommand = (
  process.env.AGENT_COMMAND ?? '../../target/debug/fireline-testy-prompt'
).split(' ')

const broker = createServer(async (req, res) => {
  if (req.method !== 'POST' || req.url !== webhook.pathname) return res.writeHead(404).end()
  const body = JSON.parse(await readBody(req)) as Record<string, string>
  await appendApprovalResolved({
    streamUrl: body.stateStreamUrl,
    sessionId: body.sessionId,
    requestId: body.requestId,
    allow: true,
    resolvedBy: 'approval-broker',
  })
  res.writeHead(202).end('approved')
})
await new Promise<void>((resolve) => broker.listen(Number(webhook.port || 8787), resolve))

const handle = await compose(
  sandbox({ labels: { demo: 'approval-broker' } }),
  middleware([trace({ includeMethods: ['session/prompt'] }), approve({ scope: 'tool_calls' })]),
  agent(agentCommand),
).start({ serverUrl, name: 'approval-broker' })

const db = createFirelineDB({ stateStreamUrl: handle.state.url })
await db.preload()

const acp = await openNodeAcpConnection(handle.acp.url, 'approval-broker')
const { sessionId } = await acp.connection.newSession({ cwd: '/', mcpServers: [] })
const resolvedPermission = waitForResolvedPermission(db, handle.state.url, sessionId)
const completedTurn = waitForCompletedTurn(db, sessionId)
await acp.connection.prompt({
  sessionId,
  prompt: [{ type: 'text', text: 'Take whatever action you need to answer this request.' }],
})

const permission = await resolvedPermission
const turn = await completedTurn

console.log(
  JSON.stringify(
    {
      message: 'approval was brokered through the stream',
      sessionId,
      permission: {
        requestId: permission.requestId,
        state: permission.state,
        outcome: permission.outcome,
      },
      turn: {
        promptTurnId: turn.promptTurnId,
        state: turn.state,
      },
    },
    null,
    2,
  ),
)

await acp.close()
db.close()
broker.close()

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Uint8Array[] = []
    req.on('data', (chunk: Uint8Array) => chunks.push(chunk))
    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')))
    req.on('error', reject)
  })
}

function waitForResolvedPermission(
  db: ReturnType<typeof createFirelineDB>,
  stateStreamUrl: string,
  sessionId: string,
) {
  return new Promise<(typeof db.collections.permissions.toArray)[number]>((resolve, reject) => {
    const sent = new Set<string>()
    const check = () => {
      const rows = db.collections.permissions.toArray
      const resolved = rows.find((row) => row.sessionId === sessionId && row.state === 'resolved')
      if (resolved) {
        clearTimeout(timeout)
        subscription.unsubscribe()
        resolve(resolved)
        return
      }
      const pending = rows.find((row) => row.sessionId === sessionId && row.state === 'pending' && !sent.has(row.requestId))
      if (!pending) return
      sent.add(pending.requestId)
      void fetch(webhookUrl, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ stateStreamUrl, sessionId, requestId: pending.requestId }),
      })
    }
    const timeout = setTimeout(() => {
      subscription.unsubscribe()
      reject(new Error('timed out waiting for approval resolution'))
    }, 5_000)
    const subscription = db.collections.permissions.subscribeChanges(check)
    check()
  })
}

function waitForCompletedTurn(db: ReturnType<typeof createFirelineDB>, sessionId: string) {
  return new Promise<(typeof db.collections.promptTurns.toArray)[number]>((resolve, reject) => {
    const check = () => {
      const completed = db.collections.promptTurns.toArray.find(
        (row) => row.sessionId === sessionId && row.state === 'completed',
      )
      if (!completed) return
      clearTimeout(timeout)
      subscription.unsubscribe()
      resolve(completed)
    }
    const timeout = setTimeout(() => {
      subscription.unsubscribe()
      reject(new Error('timed out waiting for prompt completion'))
    }, 5_000)
    const subscription = db.collections.promptTurns.subscribeChanges(check)
    check()
  })
}
