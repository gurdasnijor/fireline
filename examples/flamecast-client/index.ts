// Fireline
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, budget, inject, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'
import { createFirelineDB } from '@fireline/state'

// Third-party
import { fileURLToPath } from 'node:url'

// App code
import { openNodeAcpConnection } from '../shared/acp-node.js'
export { ReviewDashboard } from './dashboard.js'

export async function launchReviewSession(serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440') {
  const handle = await compose(
    sandbox({
      resources: [localPath(process.env.WORKSPACE_PATH ?? process.cwd(), '/workspace', true)],
      labels: { role: 'reviewer', product: 'flamecast' },
    }),
    middleware([
      trace({ includeMethods: ['session/prompt'] }),
      inject([{ kind: 'workspaceFile', path: '/workspace/README.md' }]),
      approve({ scope: 'tool_calls', timeoutMs: 120_000 }),
      budget({ tokens: 1_000_000 }),
    ]),
    agent((process.env.AGENT_COMMAND ?? 'npx -y @anthropic-ai/claude-code-acp').split(' ')),
  ).start({ serverUrl, name: 'flamecast-review' })
  const acp = await openNodeAcpConnection(handle.acp.url, 'flamecast-client')
  const session = await acp.connection.newSession({ cwd: '/workspace', mcpServers: [] })
  void acp.connection.prompt({
    sessionId: session.sessionId,
    prompt: [{ type: 'text', text: 'Review this workspace for security issues and risky dependencies.' }],
  }).catch(console.error)

  const db = createFirelineDB({ stateStreamUrl: handle.state.url })
  await db.preload()
  db.collections.promptTurns.subscribe((turns) => {
    console.log(`[flamecast] turns=${turns.filter((turn) => turn.sessionId === session.sessionId).length}`)
  })
  return { handle, sessionId: session.sessionId, db, close: () => acp.close() }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const { handle, sessionId } = await launchReviewSession()
  console.log(JSON.stringify({ sandboxId: handle.id, acpUrl: handle.acp.url, stateStreamUrl: handle.state.url, sessionId }, null, 2))
}
