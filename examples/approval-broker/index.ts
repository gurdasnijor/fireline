// Fireline
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, trace } from '@fireline/client/middleware'
import { createFirelineDB } from '@fireline/state'

// Third-party
import { createServer, type IncomingMessage } from 'node:http'

// User code
import { openNodeAcpConnection } from '../shared/acp-node.js'
import { waitForRows } from '../shared/state-subscribe.js'
import { appendApprovalResolved } from './approval.js'

const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const webhookUrl = process.env.APPROVAL_WEBHOOK ?? 'http://127.0.0.1:8787/approve'
const webhook = new URL(webhookUrl)
const agentCommand = (
  process.env.AGENT_COMMAND ?? '../../target/debug/fireline-testy-prompt'
).split(' ')

const broker = createServer(async (req, res) => {
  if (req.method !== 'POST' || req.url !== webhook.pathname) return res.writeHead(404).end()
  const body = JSON.parse(await readBody(req)) as Record<string, string>
  await appendApprovalResolved(body.stateStreamUrl, body.sessionId, body.requestId, true)
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
const sent = new Set<string>()
db.collections.permissions.subscribeChanges(() => {
  const pending = db.collections.permissions.toArray.find((row) => row.state === 'pending' && !sent.has(row.requestId))
  if (!pending) return
  sent.add(pending.requestId)
  void fetch(webhookUrl, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ stateStreamUrl: handle.state.url, sessionId: pending.sessionId, requestId: pending.requestId }),
  })
})

const acp = await openNodeAcpConnection(handle.acp.url, 'approval-broker')
const { sessionId } = await acp.connection.newSession({ cwd: '/', mcpServers: [] })
await acp.connection.prompt({ sessionId, prompt: [{ type: 'text', text: 'Take whatever action you need to answer this request.' }] })

const permissions = await waitForRows(
  db.collections.permissions,
  (rows) => rows.some((row) => row.sessionId === sessionId && row.state === 'resolved'),
  5_000,
)

console.log(
  JSON.stringify(
    {
      message: 'approval was brokered through the stream',
      sessionId,
      permissions: permissions.filter((row) => row.sessionId === sessionId).map((row) => ({ requestId: row.requestId, state: row.state, outcome: row.outcome })),
      turns: db.collections.promptTurns.toArray.filter((row) => row.sessionId === sessionId).map((row) => ({ promptTurnId: row.promptTurnId, state: row.state })),
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
