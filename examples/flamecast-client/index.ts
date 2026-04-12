// Fireline
import { Sandbox, agent, compose, sandbox } from '@fireline/client'
import { approve, budget, contextInjection, peer, trace } from '@fireline/client/middleware'
import type { ResourceRef } from '@fireline/client/resources'
import { createFirelineDB } from '@fireline/state'

// Third-party
import { fileURLToPath } from 'node:url'

// App code
import { openNodeAcpConnection } from '../shared/acp-node.js'
export { ReviewDashboard } from './dashboard.js'

export async function launchReviewSession(serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440') {
  const workspace: ResourceRef = {
    source_ref: { kind: 'localPath', host_id: 'local', path: process.env.WORKSPACE_PATH ?? process.cwd() },
    mount_path: '/workspace',
    read_only: true,
  }
  const config = compose(
    sandbox({ resources: [workspace], labels: { role: 'reviewer', product: 'flamecast' } }),
    [
      trace({ includeMethods: ['session/prompt'] }),
      contextInjection({ sources: [{ kind: 'workspaceFile', path: '/workspace/README.md' }] }),
      approve({ scope: 'tool_calls', timeoutMs: 120_000 }),
      budget({ tokens: 1_000_000 }),
      peer(),
    ],
    agent((process.env.AGENT_COMMAND ?? 'npx -y @anthropic-ai/claude-code-acp').split(' ')),
  )

  const handle = await new Sandbox({ serverUrl }).provision(config)
  const acp = await openNodeAcpConnection(handle.acp.url, 'flamecast-client')
  const session = await acp.connection.newSession({ cwd: '/workspace', mcpServers: [] })
  void acp.connection.prompt({
    sessionId: session.sessionId,
    prompt: [{ type: 'text', text: 'Review this workspace for security issues and risky dependencies.' }],
  })

  const db = createFirelineDB({ stateStreamUrl: handle.state.url })
  await db.preload()
  db.collections.promptTurns.subscribeChanges(() => {
    console.log(`[flamecast] turns=${db.collections.promptTurns.toArray.length}`)
  })
  return { handle, sessionId: session.sessionId, db, close: () => acp.close() }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const { handle, sessionId } = await launchReviewSession()
  console.log(JSON.stringify({ sandboxId: handle.id, acpUrl: handle.acp.url, stateStreamUrl: handle.state.url, sessionId }, null, 2))
}
