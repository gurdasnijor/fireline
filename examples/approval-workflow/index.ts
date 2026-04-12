import fireline, { agent, appendApprovalResolved, compose, connectAcp, middleware, sandbox } from '@fireline/client'
import { approve, secretsProxy, trace } from '@fireline/client/middleware'
import { createServer, type IncomingMessage } from 'node:http'
import { waitForRows } from '../shared/wait.js'

const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const webhookUrl = process.env.APPROVAL_WEBHOOK ?? 'http://127.0.0.1:8787/approve'
const agentCommand = (process.env.AGENT_COMMAND ?? 'npx -y @anthropic-ai/claude-code-acp').split(' ')
let streamUrl = ''; const seen = new Set<string>()
const broker = createServer(async (req, res) => { if (req.method !== 'POST' || req.url !== new URL(webhookUrl).pathname) return res.writeHead(404).end(); const body = JSON.parse(await readBody(req)) as { sessionId: string; requestId: string }; await appendApprovalResolved({ streamUrl, sessionId: body.sessionId, requestId: body.requestId, allow: true, resolvedBy: 'approval-workflow' }); res.writeHead(202).end('approved') })
await new Promise<void>((resolve) => broker.listen(Number(new URL(webhookUrl).port || 8787), resolve))
const middlewareChain = [
  trace(),
  approve({ scope: 'tool_calls' as const }),
  ...(process.env.ANTHROPIC_API_KEY
    ? [secretsProxy({ ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' } })]
    : []),
]
const handle = await compose(
  sandbox({ labels: { demo: 'approval-workflow' } }),
  middleware(middlewareChain),
  agent(agentCommand),
).start({ serverUrl, name: 'approval-workflow' }); streamUrl = handle.state.url
const db = await fireline.db({ stateStreamUrl: handle.state.url })
db.permissions.subscribe((rows) => { const pending = rows.find((row) => row.state === 'pending' && !seen.has(row.requestId)); if (!pending) return; seen.add(pending.requestId); void fetch(webhookUrl, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ sessionId: pending.sessionId, requestId: pending.requestId }) }) })
const acp = await connectAcp(handle.acp, 'approval-workflow')
const { sessionId } = await acp.newSession({ cwd: '/workspace', mcpServers: [] })
await acp.prompt({ sessionId, prompt: [{ type: 'text', text: 'Delete the build output with rm -rf dist, but wait for human approval first.' }] })
const approvals = await waitForRows(db.permissions, (rows) => rows.some((row) => row.sessionId === sessionId && row.state === 'resolved'), 15_000)
console.log(JSON.stringify({ question: 'How do I require human approval for dangerous operations?', sessionId, approvals: approvals.filter((row) => row.sessionId === sessionId).map((row) => ({ requestId: row.requestId, state: row.state, outcome: row.outcome })) }, null, 2))
await acp.close(); db.close(); broker.close()

function readBody(req: IncomingMessage) {
  return new Promise<string>((resolve, reject) => { const chunks: Uint8Array[] = []; req.on('data', (chunk: Uint8Array) => chunks.push(chunk)); req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8'))); req.on('error', reject) })
}
