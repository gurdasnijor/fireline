// Fireline
import fireline, { agent, compose, middleware, sandbox, type FirelineDB } from '@fireline/client'
import { peer } from '@fireline/client/middleware'

const agentBin = process.env.AGENT_BIN ?? '../../target/debug/fireline-testy'
const serverA = 'http://127.0.0.1:4440'
const serverB = 'http://127.0.0.1:5440'

const [agentA, agentB] = await Promise.all([
  startHarness('agent-a', serverA),
  startHarness('agent-b', serverB),
])
const db = await fireline.db({ stateStreamUrl: agentB.state.url })
const acp = await agentB.connect('cross-host-discovery')
const { sessionId } = await acp.newSession({ cwd: process.cwd(), mcpServers: [] })
await acp.prompt({ sessionId, prompt: [{ type: 'text', text: toolCall('list_peers') }] })
const peers = await observeSessionText(db, sessionId, (text) => text.includes('agent-a') && text.includes('agent-b'))
await acp.prompt({ sessionId, prompt: [{ type: 'text', text: toolCall('prompt_peer', { agentName: 'agent-a', prompt: JSON.stringify({ command: 'echo', message: 'hello across hosts' }) }) }] })
const promptPeer = await observeSessionText(db, sessionId, (text) => text.includes('agent-a') && text.includes('hello across hosts'))
await acp.close(); db.close()
console.log(JSON.stringify({ serverA, serverB, agentA: agentA.acp.url, agentB: agentB.acp.url, peers, promptPeer }, null, 2))

function startHarness(name: string, serverUrl: string) {
  return compose(sandbox(), middleware([peer()]), agent([agentBin]))
    .start({ serverUrl, name, stateStream: `cross-host-${name}` })
}

function toolCall(tool: string, params: Record<string, unknown> = {}) {
  return JSON.stringify({ command: 'call_tool', server: 'fireline-peer', tool, params })
}

function observeSessionText(
  db: FirelineDB,
  sessionId: string,
  predicate: (text: string) => boolean,
) {
  return new Promise<string>((resolve) => {
    const read = () => db.chunks.toArray.filter((chunk) => db.promptTurns.toArray.some((turn) => turn.sessionId === sessionId && turn.promptTurnId === chunk.promptTurnId)).map((chunk) => chunk.content).join('\n')
    const maybeResolve = () => { const text = read(); if (!predicate(text)) return; turns.unsubscribe(); chunks.unsubscribe(); resolve(text) }
    const turns = db.promptTurns.subscribe(maybeResolve)
    const chunks = db.chunks.subscribe(maybeResolve)
    maybeResolve()
  })
}
