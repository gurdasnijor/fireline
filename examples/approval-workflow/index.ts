import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, trace } from '@fireline/client/middleware'
import { createFirelineDB } from '@fireline/state'
import { createServer, type IncomingMessage } from 'node:http'
import { openNodeAcpConnection } from '../shared/acp-node.js'
import { resolveApproval } from '../shared/resolve-approval.js'
import { waitForRows } from '../shared/wait.js'

const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const webhookUrl = process.env.APPROVAL_WEBHOOK ?? 'http://127.0.0.1:8787/approve'
const envVars = process.env.ANTHROPIC_API_KEY ? { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY } : undefined
const agentCommand = (process.env.AGENT_COMMAND ?? 'npx -y @anthropic-ai/claude-code-acp').split(' ')
let streamUrl = ''; const seen = new Set<string>()
const broker = createServer(async (req, res) => { if (req.method !== 'POST' || req.url !== new URL(webhookUrl).pathname) return res.writeHead(404).end(); const body = JSON.parse(await readBody(req)) as { sessionId: string; requestId: string }; await resolveApproval(streamUrl, body.sessionId, body.requestId, true); res.writeHead(202).end('approved') })
await new Promise<void>((resolve) => broker.listen(Number(new URL(webhookUrl).port || 8787), resolve))
const handle = await compose(sandbox({ envVars, labels: { demo: 'approval-workflow' } }), middleware([trace(), approve({ scope: 'tool_calls' })]), agent(agentCommand)).start({ serverUrl, name: 'approval-workflow' }); streamUrl = handle.state.url
const db = createFirelineDB({ stateStreamUrl: handle.state.url }); await db.preload()
db.collections.permissions.subscribeChanges(() => { const pending = db.collections.permissions.toArray.find((row) => row.state === 'pending' && !seen.has(row.requestId)); if (!pending) return; seen.add(pending.requestId); void fetch(webhookUrl, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ sessionId: pending.sessionId, requestId: pending.requestId }) }) })
const acp = await openNodeAcpConnection(handle.acp.url, 'approval-workflow')
const { sessionId } = await acp.connection.newSession({ cwd: '/workspace', mcpServers: [] })
await acp.connection.prompt({ sessionId, prompt: [{ type: 'text', text: 'Delete the build output with rm -rf dist, but wait for human approval first.' }] })
const approvals = await waitForRows(db.collections.permissions, (rows) => rows.some((row) => row.sessionId === sessionId && row.state === 'resolved'), 15_000)
console.log(JSON.stringify({ question: 'How do I require human approval for dangerous operations?', sessionId, approvals: approvals.filter((row) => row.sessionId === sessionId).map((row) => ({ requestId: row.requestId, state: row.state, outcome: row.outcome })) }, null, 2))
await acp.close(); db.close(); broker.close()

function readBody(req: IncomingMessage) {
  return new Promise<string>((resolve, reject) => { const chunks: Uint8Array[] = []; req.on('data', (chunk: Uint8Array) => chunks.push(chunk)); req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8'))); req.on('error', reject) })
}
