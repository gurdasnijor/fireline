// Fireline
import { agent, compose, middleware, peer, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'
import { createFirelineDB } from '@fireline/state'

// Third-party
import { fileURLToPath } from 'node:url'

// App code
import { openNodeAcpConnection } from '../shared/acp-node.js'

const workspace = localPath(process.env.WORKSPACE_PATH ?? process.cwd(), '/workspace', true)
const reviewer = compose(sandbox({ resources: [workspace] }), middleware([trace()]), agent((process.env.AGENT_COMMAND ?? 'npx -y @anthropic-ai/claude-code-acp').split(' '))).as('reviewer')
const writer = compose(sandbox({ resources: [workspace] }), middleware([trace()]), agent((process.env.AGENT_COMMAND ?? 'npx -y @anthropic-ai/claude-code-acp').split(' '))).as('writer')

export async function launchAgentOs(localUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440', remoteUrl = process.env.REMOTE_FIRELINE_URL ?? localUrl) {
  const handles = await peer(reviewer, writer).start({ serverUrl: localUrl, stateStream: 'agent-os-demo' })
  const local = await openNodeAcpConnection(handles.reviewer.acp.url, 'agent-os-local')
  const { sessionId } = await local.connection.newSession({ cwd: '/workspace', mcpServers: [] })
  await local.connection.prompt({
    sessionId,
    prompt: [{ type: 'text', text: 'Analyze this codebase and ask the writer peer for a short release note draft.' }],
  })

  const remoteHandle = await compose(
    sandbox({ resources: [workspace] }),
    middleware([trace()]),
    agent((process.env.AGENT_COMMAND ?? 'npx -y @anthropic-ai/claude-code-acp').split(' ')),
  ).start({ serverUrl: remoteUrl, name: 'remote-reviewer', stateStream: 'agent-os-demo' })
  const remote = await openNodeAcpConnection(remoteHandle.acp.url, 'agent-os-remote')
  await remote.connection.loadSession({ sessionId, cwd: '/workspace', mcpServers: [] })
  await remote.connection.prompt({
    sessionId,
    prompt: [{ type: 'text', text: 'Continue from the existing session and summarize the remaining risks.' }],
  })

  const db = createFirelineDB({ stateStreamUrl: handles.reviewer.state.url })
  await db.preload()
  db.collections.promptTurns.subscribe((turns) => console.log(`[agent-os] turns=${turns.filter((turn) => turn.sessionId === sessionId).length}`))
  db.collections.childSessionEdges.subscribe((edges) => console.log(`[agent-os] peer-calls=${edges.length}`))
  return { handles, remoteHandle, sessionId }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  console.log(JSON.stringify(await launchAgentOs(), null, 2))
}
