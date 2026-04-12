// Fireline
import { Sandbox, agent, compose, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'
import { createFirelineDB } from '@fireline/state'

// User code
import { openNodeAcpConnection } from '../shared/acp-node.js'
import { localPathResource } from '../_shared/resources.js'
import { waitFor } from '../_shared/wait.js'

const localUrl = process.env.FIRELINE_LOCAL_URL ?? 'http://127.0.0.1:4440'
const remoteUrl = process.env.FIRELINE_REMOTE_URL ?? 'http://127.0.0.1:5440'
const stateStream = process.env.STATE_STREAM ?? `demo-session-migration-${Date.now()}`
const agentCommand = (
  process.env.AGENT_COMMAND ?? '../../target/debug/fireline-testy-load'
).split(' ')

const config = compose(
  sandbox({
    resources: [localPathResource(process.env.WORKSPACE_PATH ?? process.cwd())],
    labels: { demo: 'session-migration' },
  }),
  [trace({ includeMethods: ['session/new', 'session/load', 'session/prompt'] })],
  agent(agentCommand),
)

const localHandle = await new Sandbox({ serverUrl: localUrl }).provision({
  ...config,
  name: 'session-migration-local',
  stateStream,
})
const db = createFirelineDB({ stateStreamUrl: localHandle.state.url })
await db.preload()

const localAcp = await openNodeAcpConnection(
  import.meta.url,
  localHandle.acp.url,
  'session-migration-local',
)
const { sessionId } = await localAcp.connection.newSession({ cwd: '/workspace', mcpServers: [] })
await localAcp.connection.prompt({ sessionId, prompt: [{ type: 'text', text: 'turn 1 on localhost' }] })
await localAcp.connection.prompt({ sessionId, prompt: [{ type: 'text', text: 'turn 2 on localhost' }] })

const remoteHandle = await new Sandbox({ serverUrl: remoteUrl }).provision({
  ...config,
  name: 'session-migration-remote',
  stateStream,
})
const remoteAcp = await openNodeAcpConnection(
  import.meta.url,
  remoteHandle.acp.url,
  'session-migration-remote',
)
await remoteAcp.connection.loadSession({ sessionId, cwd: '/workspace', mcpServers: [] })
await remoteAcp.connection.prompt({ sessionId, prompt: [{ type: 'text', text: 'turn 3 on remote' }] })

await waitFor(
  () =>
    db.collections.promptTurns.toArray.filter(
      (turn) => turn.sessionId === sessionId && turn.state === 'completed',
    ).length >= 3
      ? true
      : undefined,
  5_000,
)

console.log(
  JSON.stringify(
    {
      message: 'the session lived in the stream, not in the sandbox',
      sessionId,
      stateStreamUrl: localHandle.state.url,
      runtimeInstances: db.collections.runtimeInstances.toArray.map((row) => row.instanceId),
      turns: db.collections.promptTurns.toArray
        .filter((turn) => turn.sessionId === sessionId)
        .map((turn) => ({ promptTurnId: turn.promptTurnId, text: turn.text, state: turn.state })),
    },
    null,
    2,
  ),
)

await localAcp.close()
await remoteAcp.close()
db.close()
