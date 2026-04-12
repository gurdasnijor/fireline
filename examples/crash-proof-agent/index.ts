import { agent, compose, middleware, sandbox } from '@fireline/client'
import { SandboxAdmin } from '@fireline/client/admin'
import { trace } from '@fireline/client/middleware'
import { createFirelineDB } from '@fireline/state'
import { openNodeAcpConnection } from '../shared/acp-node.js'
import { waitForRows } from '../shared/wait.js'

const primaryUrl = process.env.FIRELINE_PRIMARY_URL ?? 'http://127.0.0.1:4440'
const rescueUrl = process.env.FIRELINE_RESCUE_URL ?? 'http://127.0.0.1:5440'
const stateStream = process.env.STATE_STREAM ?? `crash-proof-${Date.now()}`
const harness = compose(sandbox({ labels: { demo: 'crash-proof-agent' } }), middleware([trace({ includeMethods: ['session/new', 'session/load', 'session/prompt'] })]), agent((process.env.AGENT_COMMAND ?? '../../target/debug/fireline-testy-load').split(' ')))
const first = await harness.start({ serverUrl: primaryUrl, name: 'crash-proof-primary', stateStream })
const acp1 = await openNodeAcpConnection(first.acp.url, 'crash-proof-primary')
const { sessionId } = await acp1.connection.newSession({ cwd: '/workspace', mcpServers: [] })
await acp1.connection.prompt({ sessionId, prompt: [{ type: 'text', text: 'Turn one: start auditing the repo and keep going after a crash.' }] })
await new SandboxAdmin({ serverUrl: primaryUrl }).destroy(first.id); await acp1.close()
const second = await harness.start({ serverUrl: rescueUrl, name: 'crash-proof-rescue', stateStream })
const db = createFirelineDB({ stateStreamUrl: second.state.url }); await db.preload()
const acp2 = await openNodeAcpConnection(second.acp.url, 'crash-proof-rescue')
await acp2.connection.loadSession({ sessionId, cwd: '/workspace', mcpServers: [] })
await acp2.connection.prompt({ sessionId, prompt: [{ type: 'text', text: 'Turn two: finish the audit without repeating yourself.' }] })
const turns = await waitForRows(db.collections.promptTurns, (rows) => rows.filter((row) => row.sessionId === sessionId && row.state === 'completed').length >= 2, 10_000)
console.log(JSON.stringify({ question: 'What happens when the agent crashes mid-task?', sessionId, firstSandboxId: first.id, secondSandboxId: second.id, turns: turns.filter((row) => row.sessionId === sessionId).map((row) => row.text) }, null, 2))
await acp2.close(); db.close()
