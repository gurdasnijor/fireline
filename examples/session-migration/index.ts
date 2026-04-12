// Fireline
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'
import { createFirelineDB } from '@fireline/state'

// Third-party

// User code
import { openNodeAcpConnection } from '../shared/acp-node.js'

const localUrl = process.env.FIRELINE_LOCAL_URL ?? 'http://127.0.0.1:4440'
const remoteUrl = process.env.FIRELINE_REMOTE_URL ?? 'http://127.0.0.1:5440'
const stateStream = process.env.STATE_STREAM ?? `demo-session-migration-${Date.now()}`
const agentCommand = (
  process.env.AGENT_COMMAND ?? '../../target/debug/fireline-testy-load'
).split(' ')

const harness = compose(
  sandbox({
    resources: [localPath(process.env.WORKSPACE_PATH ?? process.cwd(), '/workspace', true)],
    labels: { demo: 'session-migration' },
  }),
  middleware([trace({ includeMethods: ['session/new', 'session/load', 'session/prompt'] })]),
  agent(agentCommand),
)

const localHandle = await harness.start({
  serverUrl: localUrl,
  name: 'session-migration-local',
  stateStream,
})
const db = createFirelineDB({ stateStreamUrl: localHandle.state.url })
await db.preload()

const localAcp = await openNodeAcpConnection(localHandle.acp.url, 'session-migration-local')
const { sessionId } = await localAcp.connection.newSession({ cwd: '/workspace', mcpServers: [] })
const completedTurns = waitForCompletedTurns(db, sessionId, 3)
await localAcp.connection.prompt({ sessionId, prompt: [{ type: 'text', text: 'turn 1 on localhost' }] })
await localAcp.connection.prompt({ sessionId, prompt: [{ type: 'text', text: 'turn 2 on localhost' }] })

const remoteHandle = await harness.start({
  serverUrl: remoteUrl,
  name: 'session-migration-remote',
  stateStream,
})
const remoteAcp = await openNodeAcpConnection(remoteHandle.acp.url, 'session-migration-remote')
await remoteAcp.connection.loadSession({ sessionId, cwd: '/workspace', mcpServers: [] })
await remoteAcp.connection.prompt({ sessionId, prompt: [{ type: 'text', text: 'turn 3 on remote' }] })
await completedTurns

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

function waitForCompletedTurns(
  db: ReturnType<typeof createFirelineDB>,
  sessionId: string,
  count: number,
) {
  return new Promise<(typeof db.collections.promptTurns.toArray)>((resolve, reject) => {
    const check = () => {
      const rows = db.collections.promptTurns.toArray
      const completed = rows.filter((turn) => turn.sessionId === sessionId && turn.state === 'completed')
      if (completed.length < count) return
      clearTimeout(timeout)
      subscription.unsubscribe()
      resolve(rows)
    }
    const timeout = setTimeout(() => {
      subscription.unsubscribe()
      reject(new Error('timed out waiting for migrated session turns'))
    }, 5_000)
    const subscription = db.collections.promptTurns.subscribeChanges(check)
    check()
  })
}
