import { agent, compose, middleware, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'
import { createFirelineDB } from '@fireline/state'
import { openNodeAcpConnection } from '../shared/acp-node.js'

const stateStreamUrl = process.env.TASK_STREAM_URL
if (stateStreamUrl) {
  const db = createFirelineDB({ stateStreamUrl }); await db.preload()
  console.log(JSON.stringify({ question: 'Can I fire off an agent and check on it later?', sessions: db.collections.sessions.toArray.map((row) => row.sessionId), turns: db.collections.promptTurns.toArray.length, latestText: db.collections.promptTurns.toArray.at(-1)?.text }, null, 2))
  db.close(); process.exit(0)
}

const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const envVars = process.env.ANTHROPIC_API_KEY ? { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY } : undefined
const agentCommand = (process.env.AGENT_COMMAND ?? 'npx -y @anthropic-ai/claude-code-acp').split(' ')
const handle = await compose(sandbox({ envVars, labels: { demo: 'background-task' } }), middleware([trace()]), agent(agentCommand)).start({ serverUrl, name: 'background-task' })
const acp = await openNodeAcpConnection(handle.acp.url, 'background-task')
const { sessionId } = await acp.connection.newSession({ cwd: '/workspace', mcpServers: [] })
await acp.connection.prompt({ sessionId, prompt: [{ type: 'text', text: process.env.TASK_PROMPT ?? 'Audit this repository overnight and leave a morning summary with the top three risks.' }] })
console.log(JSON.stringify({ question: 'Can I fire off an agent and check on it later?', taskId: handle.id, sessionId, stateStream: handle.state.url }, null, 2))
await acp.close()
