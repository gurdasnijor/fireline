import fireline, { agent, compose, middleware, sandbox } from '@fireline/client'
import { secretsProxy, trace } from '@fireline/client/middleware'

const stateStreamUrl = process.env.TASK_STREAM_URL
if (stateStreamUrl) {
  const db = await fireline.db({ stateStreamUrl })
  console.log(JSON.stringify({ question: 'Can I fire off an agent and check on it later?', sessions: db.sessions.toArray.map((row) => row.sessionId), turns: db.promptRequests.toArray.length, latestText: db.promptRequests.toArray.at(-1)?.text }, null, 2))
  db.close(); process.exit(0)
}

const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const agentCommand = (process.env.AGENT_COMMAND ?? 'npx -y @anthropic-ai/claude-code-acp').split(' ')
const middlewareChain = [
  trace(),
  ...(process.env.ANTHROPIC_API_KEY
    ? [secretsProxy({ ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' } })]
    : []),
]
const handle = await compose(
  sandbox({ labels: { demo: 'background-task' } }),
  middleware(middlewareChain),
  agent(agentCommand),
).start({ serverUrl, name: 'background-task' })
const acp = await handle.connect('background-task')
const { sessionId } = await acp.newSession({ cwd: '/workspace', mcpServers: [] })
await acp.prompt({ sessionId, prompt: [{ type: 'text', text: process.env.TASK_PROMPT ?? 'Audit this repository overnight and leave a morning summary with the top three risks.' }] })
console.log(JSON.stringify({ question: 'Can I fire off an agent and check on it later?', taskId: handle.id, sessionId, stateStream: handle.state.url }, null, 2))
await acp.close()
