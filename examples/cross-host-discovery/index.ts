import fireline, { agent, compose, middleware, sandbox } from '@fireline/client'
import { peer, trace } from '@fireline/client/middleware'
import { extractChunkTextPreview } from '@fireline/state'

const controlPlaneA = process.env.FIRELINE_REGION_A_URL ?? 'http://127.0.0.1:4440'
const controlPlaneB = process.env.FIRELINE_REGION_B_URL ?? 'http://127.0.0.1:5440'
const demoId = process.env.DEMO_ID ?? `${Date.now()}`
const demoSuffix = demoId.replace(/[^a-zA-Z0-9]/g, '').slice(-6)
const callerName = process.env.CALLER_AGENT_NAME ?? `dispatcher-east-${demoSuffix}`
const calleeName = process.env.CALLEE_AGENT_NAME ?? `inventory-west-${demoSuffix}`
const callerStateStream = process.env.CALLER_STATE_STREAM ?? `cross-host-${demoId}-caller`
const calleeStateStream = process.env.CALLEE_STATE_STREAM ?? `cross-host-${demoId}-callee`
const agentCommand = splitCommand(
  process.env.AGENT_COMMAND ?? '../../target/debug/fireline-testy',
)
const observationTimeoutMs = Number(process.env.OBSERVATION_TIMEOUT_MS ?? 10_000)
const remoteMessage =
  process.env.REMOTE_MESSAGE ??
  'order 4815 is reserved in the west warehouse and can ship today'

const [callee, caller] = await Promise.all([
  startHarness({
    name: calleeName,
    serverUrl: controlPlaneA,
    stateStream: calleeStateStream,
  }),
  startHarness({
    name: callerName,
    serverUrl: controlPlaneB,
    stateStream: callerStateStream,
  }),
])

let callerConnection: Awaited<ReturnType<typeof caller.connect>> | null = null
let callerDb: Awaited<ReturnType<typeof fireline.db>> | null = null
let calleeDb: Awaited<ReturnType<typeof fireline.db>> | null = null

try {
  callerDb = await fireline.db({ stateStreamUrl: caller.state.url })
  calleeDb = await fireline.db({ stateStreamUrl: callee.state.url })
  callerConnection = await caller.connect('cross-host-discovery')

  const { sessionId } = await callerConnection.newSession({
    cwd: process.cwd(),
    mcpServers: [],
  })

  await callerConnection.prompt({
    sessionId,
    prompt: [{ type: 'text', text: toolCall('list_peers') }],
  })

  await waitForCondition({
    collection: callerDb.chunks,
    timeoutMs: observationTimeoutMs,
    description: 'list_peers to materialize both runtimes in the caller transcript',
    predicate: () => {
      const sessionText = readAllChunksText(callerDb!)
      return sessionText.includes(callerName) && sessionText.includes(calleeName)
    },
  })

  const listPeersText = readAllChunksText(callerDb)

  await callerConnection.prompt({
    sessionId,
    prompt: [
      {
        type: 'text',
        text: toolCall('prompt_peer', {
          agentName: calleeName,
          prompt: JSON.stringify({ command: 'echo', message: remoteMessage }),
        }),
      },
    ],
  })

  await Promise.all([
    waitForCondition({
      collection: callerDb.chunks,
      timeoutMs: observationTimeoutMs,
      description: 'caller stream to record the completed remote handoff',
      predicate: () => readAllChunksText(callerDb!).includes(remoteMessage),
    }),
    waitForCondition({
      collection: calleeDb.chunks,
      timeoutMs: observationTimeoutMs,
      description: 'callee stream to record the delegated prompt',
      predicate: () => readAllChunksText(calleeDb!).includes(remoteMessage),
    }),
  ])

  const calleeTranscriptText = readAllChunksText(calleeDb)
  const calleeSession =
    [...calleeDb.sessions.toArray].sort((left, right) => left.createdAt - right.createdAt).at(-1) ??
    null
  const peersVisible = [callerName, calleeName].filter((name) => listPeersText.includes(name))

  console.log(
    JSON.stringify(
      {
        question:
          'Can agents on different hosts find each other through the stream instead of a service registry?',
        topology: {
          sharedDiscoveryPlane: 'Both control planes publish into the same durable-streams deployment.',
          controlPlaneA,
          controlPlaneB,
          callerStateStream: caller.state.url,
          calleeStateStream: callee.state.url,
        },
        peersVisible,
        callerSessionId: sessionId,
        calleeSessionId: calleeSession?.sessionId ?? null,
        remoteHandoff: {
          target: calleeName,
          requestedMessage: remoteMessage,
          responseText: calleeTranscriptText.includes(remoteMessage)
            ? remoteMessage
            : null,
        },
        callerTranscriptExcerpt: listPeersText,
        calleeTranscriptExcerpt: calleeTranscriptText,
        stateEvidence: {
          callerDiscoveredBothPeers: peersVisible.length === 2,
          calleeCompletedTurns: calleeDb.promptRequests.toArray
            .filter((row) => row.state === 'completed')
            .map((row) => ({
              sessionId: row.sessionId,
              requestId: row.requestId,
              text: row.text,
            })),
        },
      },
      null,
      2,
    ),
  )
} finally {
  await callerConnection?.close().catch(() => {})
  callerDb?.close()
  calleeDb?.close()
}

function startHarness(options: {
  readonly name: string
  readonly serverUrl: string
  readonly stateStream: string
}) {
  return compose(
    sandbox({
      labels: {
        demo: 'cross-host-discovery',
        example: 'cross-host-discovery',
        runtime: options.name,
      },
    }),
    middleware([trace(), peer()]),
    agent(agentCommand),
  ).start({
    serverUrl: options.serverUrl,
    name: options.name,
    stateStream: options.stateStream,
  })
}

function toolCall(tool: string, params: Record<string, unknown> = {}) {
  return JSON.stringify({
    command: 'call_tool',
    server: 'fireline-peer',
    tool,
    params,
  })
}

function splitCommand(command: string): string[] {
  return command
    .split(' ')
    .map((part) => part.trim())
    .filter((part) => part.length > 0)
}

function readAllChunksText(db: Awaited<ReturnType<typeof fireline.db>>): string {
  return db.chunks.toArray
    .sort((left, right) => left.createdAt - right.createdAt)
    .map((row) => extractChunkTextPreview(row.update))
    .join('')
}

async function waitForCondition<T extends object>(options: {
  readonly collection: {
    readonly subscribe: (callback: (rows: T[]) => void) => { unsubscribe(): void }
    readonly toArray: readonly T[]
  }
  readonly timeoutMs: number
  readonly description: string
  readonly predicate: () => boolean
}) {
  const { collection, timeoutMs, description, predicate } = options

  if (predicate()) {
    return
  }

  await new Promise<void>((resolve, reject) => {
    const timeout = setTimeout(() => {
      subscription.unsubscribe()
      reject(new Error(`timed out after ${timeoutMs}ms waiting for ${description}`))
    }, timeoutMs)

    const finish = () => {
      clearTimeout(timeout)
      subscription.unsubscribe()
      resolve()
    }

    const subscription = collection.subscribe(() => {
      if (predicate()) {
        finish()
      }
    })
  })
}
