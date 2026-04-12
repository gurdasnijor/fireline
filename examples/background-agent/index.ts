// Fireline
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, budget, trace } from '@fireline/client/middleware'
import { gitRepo } from '@fireline/client/resources'

// Third-party
import { fileURLToPath } from 'node:url'

// App code
import { openNodeAcpConnection } from '../shared/acp-node.js'
export { TaskDashboard } from './dashboard.js'

export async function launchBackgroundTask(serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440') {
  const taskId = process.env.TASK_ID ?? `task-${Date.now()}`
  const handle = await compose(
    sandbox({
      resources: [gitRepo(process.env.REPO_URL ?? 'https://github.com/fireline-rs/fireline', process.env.REPO_REF ?? 'main', '/workspace')],
      labels: { task: taskId, owner: process.env.USER ?? 'background-agent' },
    }),
    middleware([trace(), approve({ scope: 'tool_calls', timeoutMs: 300_000 }), budget({ tokens: 2_000_000 })]),
    agent((process.env.AGENT_COMMAND ?? 'npx -y @anthropic-ai/claude-code-acp').split(' ')),
  ).start({ serverUrl, name: taskId })
  const acp = await openNodeAcpConnection(handle.acp.url, 'background-agent')
  const { sessionId } = await acp.connection.newSession({ cwd: '/workspace', mcpServers: [] })
  void acp.connection.prompt({
    sessionId,
    prompt: [{ type: 'text', text: process.env.TASK_PROMPT ?? 'Review this repository and prepare a migration plan.' }],
  }).catch(console.error)
  setTimeout(() => void acp.close().catch(console.error), 1_000)
  return { taskId, handle, sessionId }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  console.log(JSON.stringify(await launchBackgroundTask(), null, 2))
}
