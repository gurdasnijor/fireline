import fireline, { agent, compose, connectAcp, middleware, sandbox } from '@fireline/client'
import { approve, secretsProxy, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'
import { waitForRows } from '../shared/wait.js'

const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const repoPath = process.env.REPO_PATH ?? process.cwd()
const agentCommand = (process.env.AGENT_COMMAND ?? 'npx -y @anthropic-ai/claude-code-acp').split(' ')
const middlewareChain = [
  trace(),
  approve({ scope: 'tool_calls' as const }),
  ...(process.env.ANTHROPIC_API_KEY
    ? [secretsProxy({ ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' } })]
    : []),
]
const handle = await compose(
  sandbox({ resources: [localPath(repoPath, '/workspace')], labels: { demo: 'code-review-agent' } }),
  middleware(middlewareChain),
  agent(agentCommand),
).start({ serverUrl, name: 'code-review-agent' })
const db = await fireline.db({ stateStreamUrl: handle.state.url })
const acp = await connectAcp(handle.acp, 'code-review-agent')
const { sessionId } = await acp.newSession({ cwd: '/workspace', mcpServers: [] })
await acp.prompt({ sessionId, prompt: [{ type: 'text', text: 'Review this repo, explain the problems you find, and ask for approval before every file write.' }] })
const approvals = await waitForRows(db.permissions, (rows) => rows.some((row) => row.sessionId === sessionId && row.state === 'pending'), 15_000)
console.log(JSON.stringify({ question: 'Can an AI review my code and ask permission before making changes?', repoPath, sessionId, pendingApprovals: approvals.filter((row) => row.sessionId === sessionId).length, stateStream: handle.state.url }, null, 2))
await acp.close(); db.close()
