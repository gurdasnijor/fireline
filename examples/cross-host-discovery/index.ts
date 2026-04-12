// Fireline
import fireline, { agent, compose, middleware, sandbox } from '@fireline/client'
import { peer } from '@fireline/client/middleware'
import { extractChunkTextPreview } from '@fireline/state'
import { waitForRows } from '../shared/wait.ts'

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
const readSessionText = () => {
  const requestIds = new Set(
    db.promptTurns.toArray
      .filter((turn) => turn.sessionId === sessionId)
      .map((turn) => turn.requestId),
  )
  return db.chunks.toArray
    .filter((chunk) => requestIds.has(chunk.requestId))
    .map((chunk) => extractChunkTextPreview(chunk.update))
    .join('\n')
}
await acp.prompt({ sessionId, prompt: [{ type: 'text', text: toolCall('list_peers') }] })
await waitForRows(db.chunks, () => readSessionText().includes('agent-a') && readSessionText().includes('agent-b'), 10_000)
const peers = readSessionText()
await acp.prompt({ sessionId, prompt: [{ type: 'text', text: toolCall('prompt_peer', { agentName: 'agent-a', prompt: JSON.stringify({ command: 'echo', message: 'hello across hosts' }) }) }] })
await waitForRows(db.chunks, () => readSessionText().includes('agent-a') && readSessionText().includes('hello across hosts'), 10_000)
const promptPeer = readSessionText()
await acp.close(); db.close()
console.log(JSON.stringify({ serverA, serverB, agentA: agentA.acp.url, agentB: agentB.acp.url, peers, promptPeer }, null, 2))

function startHarness(name: string, serverUrl: string) {
  return compose(sandbox(), middleware([peer()]), agent([agentBin]))
    .start({ serverUrl, name, stateStream: `cross-host-${name}` })
}

function toolCall(tool: string, params: Record<string, unknown> = {}) {
  return JSON.stringify({ command: 'call_tool', server: 'fireline-peer', tool, params })
}
