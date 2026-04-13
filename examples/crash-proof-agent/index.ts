import fireline, { agent, compose, middleware, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'

const primaryUrl = process.env.FIRELINE_PRIMARY_URL ?? 'http://127.0.0.1:4440'
const rescueUrl = process.env.FIRELINE_RESCUE_URL ?? 'http://127.0.0.1:5440'
const stateStream = process.env.STATE_STREAM ?? `crash-proof-${Date.now()}`
const workingDirectory = process.env.WORKSPACE_DIR ?? '/workspace'
const firstPrompt =
  process.env.FIRST_PROMPT ??
  'Start a release-readiness review for this repository. Keep track of the risks you find because another host may need to pick this work up later.'
const secondPrompt =
  process.env.SECOND_PROMPT ??
  'You are now running on a replacement host. Continue the same review without starting over and finish with the top three release blockers.'
const observationTimeoutMs = Number(process.env.OBSERVATION_TIMEOUT_MS ?? 10_000)

const harness = compose(
  sandbox({
    labels: { demo: 'crash-proof-agent', example: 'crash-proof-agent' },
  }),
  middleware([
    trace({
      includeMethods: ['session/new', 'session/prompt', 'session/load'],
    }),
  ]),
  agent(
    splitCommand(
      process.env.AGENT_COMMAND ?? '../../target/debug/fireline-testy-load',
    ),
  ),
)

const first = await harness.start({
  serverUrl: primaryUrl,
  name: 'crash-proof-primary',
  stateStream,
})

let second:
  | Awaited<ReturnType<typeof harness.start>>
  | null = null
let firstConnection: Awaited<ReturnType<typeof first.connect>> | null = null
let secondConnection: Awaited<ReturnType<typeof first.connect>> | null = null
let db: Awaited<ReturnType<typeof fireline.db>> | null = null

try {
  firstConnection = await first.connect('crash-proof-primary')
  const { sessionId } = await firstConnection.newSession({
    cwd: workingDirectory,
    mcpServers: [],
  })

  await firstConnection.prompt({
    sessionId,
    prompt: [{ type: 'text', text: firstPrompt }],
  })
  await firstConnection.close()
  firstConnection = null

  // The handoff is intentionally controlled here: we stop the first runtime to
  // prove that session identity and transcript state live on the stream rather
  // than inside the original sandbox process.
  await first.stop()

  second = await harness.start({
    serverUrl: rescueUrl,
    name: 'crash-proof-rescue',
    stateStream,
  })
  secondConnection = await second.connect('crash-proof-rescue')
  db = await fireline.db({ stateStreamUrl: second.state.url })

  await secondConnection.loadSession({
    sessionId,
    cwd: workingDirectory,
    mcpServers: [],
  })
  await secondConnection.prompt({
    sessionId,
    prompt: [{ type: 'text', text: secondPrompt }],
  })

  const promptRequests = await observePromptRequests({
    db,
    sessionId,
    timeoutMs: observationTimeoutMs,
  })
  const sessionRow =
    db.sessions.toArray.find((row) => row.sessionId === sessionId) ?? null

  console.log(
    JSON.stringify(
      {
        question: 'Can a replacement host continue the same agent session?',
        primaryHost: primaryUrl,
        rescueHost: rescueUrl,
        stateStream,
        sessionId,
        firstSandboxId: first.id,
        secondSandboxId: second.id,
        supportsLoadSession: sessionRow?.supportsLoadSession ?? null,
        promptRequests,
      },
      null,
      2,
    ),
  )
} finally {
  await secondConnection?.close().catch(() => {})
  await firstConnection?.close().catch(() => {})
  db?.close()
}

function splitCommand(command: string): string[] {
  return command
    .split(' ')
    .map((part) => part.trim())
    .filter((part) => part.length > 0)
}

async function observePromptRequests(options: {
  readonly db: Awaited<ReturnType<typeof fireline.db>>
  readonly sessionId: string
  readonly timeoutMs: number
}) {
  const { db, sessionId, timeoutMs } = options

  return await new Promise<
    Array<{
      readonly requestId: string | number | null
      readonly state: string
      readonly text?: string
    }>
  >((resolve, reject) => {
    let timeout: ReturnType<typeof setTimeout> | null = null
    let subscription:
      | ReturnType<(typeof db.promptRequests)['subscribe']>
      | null = null

    const finish = (
      result: Array<{
        readonly requestId: string | number | null
        readonly state: string
        readonly text?: string
      }>,
    ) => {
      if (timeout) {
        clearTimeout(timeout)
      }
      subscription?.unsubscribe()
      resolve(result)
    }

    const fail = (error: Error) => {
      if (timeout) {
        clearTimeout(timeout)
      }
      subscription?.unsubscribe()
      reject(error)
    }

    timeout = setTimeout(() => {
      fail(
        new Error(
          `timed out after ${timeoutMs}ms waiting for two completed prompt requests in session ${sessionId}`,
        ),
      )
    }, timeoutMs)

    subscription = db.promptRequests.subscribe((rows) => {
      const sessionRows = rows.filter((row) => row.sessionId === sessionId)
      const completedRows = sessionRows.filter((row) => row.state === 'completed')
      if (completedRows.length < 2) {
        return
      }

      finish(
        sessionRows.map((row) => ({
          requestId: row.requestId,
          state: row.state,
          text: row.text,
        })),
      )
    })
  })
}
