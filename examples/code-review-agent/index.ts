import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'
import { createFirelineDB } from '@fireline/state'
import { openNodeAcpConnection } from '../shared/acp-node.js'
import { waitForRows } from '../shared/wait.js'

const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const repoPath = process.env.REPO_PATH ?? process.cwd()
const envVars = process.env.ANTHROPIC_API_KEY ? { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY } : undefined
const agentCommand = (process.env.AGENT_COMMAND ?? 'npx -y @anthropic-ai/claude-code-acp').split(' ')
const handle = await compose(sandbox({ resources: [localPath(repoPath, '/workspace')], envVars, labels: { demo: 'code-review-agent' } }), middleware([trace(), approve({ scope: 'tool_calls' })]), agent(agentCommand)).start({ serverUrl, name: 'code-review-agent' })
const db = createFirelineDB({ stateStreamUrl: handle.state.url }); await db.preload()
const acp = await openNodeAcpConnection(handle.acp.url, 'code-review-agent')
const { sessionId } = await acp.connection.newSession({ cwd: '/workspace', mcpServers: [] })
await acp.connection.prompt({ sessionId, prompt: [{ type: 'text', text: 'Review this repo, explain the problems you find, and ask for approval before every file write.' }] })
const approvals = await waitForRows(db.collections.permissions, (rows) => rows.some((row) => row.sessionId === sessionId && row.state === 'pending'), 15_000)
console.log(JSON.stringify({ question: 'Can an AI review my code and ask permission before making changes?', repoPath, sessionId, pendingApprovals: approvals.filter((row) => row.sessionId === sessionId).length, stateStream: handle.state.url }, null, 2))
await acp.close(); db.close()
