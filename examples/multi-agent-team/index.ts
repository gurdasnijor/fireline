import { randomUUID } from 'node:crypto'

import fireline, {
  agent,
  compose,
  fanout,
  middleware,
  peer,
  pipe,
  sandbox,
  type ConnectedAcp,
  type FirelineAgent,
  type FirelineDB,
  type RequestId,
  type SessionId,
} from '@fireline/client'
import { peer as peerMiddleware, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'
import { extractChunkTextPreview, type PromptRequestRow } from '@fireline/state'

const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const sharedStateStream =
  process.env.STATE_STREAM ?? `multi-agent-team-${randomUUID()}`
const reviewerCount = Number(process.env.REVIEWER_COUNT ?? '3')
const reviewerBaseName = 'reviewer'
const commandMode =
  process.env.TEAM_COMMAND_MODE === 'testy' ? 'testy' : 'natural'
const workspacePath = process.env.WORKSPACE_PATH ?? process.cwd()
const workspace = localPath(workspacePath, '/workspace', true)
const agentCommand = splitCommand(
  process.env.AGENT_COMMAND ?? 'npx -y @agentclientprotocol/claude-agent-acp',
)

const researcher = compose(
  sandbox({
    resources: [workspace],
    labels: { demo: 'multi-agent-team', topology: 'pipe', role: 'researcher' },
  }),
  middleware([trace()]),
  agent(agentCommand),
).as('researcher')

const writer = compose(
  sandbox({
    resources: [workspace],
    labels: { demo: 'multi-agent-team', topology: 'pipe', role: 'writer' },
  }),
  middleware([trace()]),
  agent(agentCommand),
).as('writer')

const reviewer = compose(
  sandbox({
    resources: [workspace],
    labels: { demo: 'multi-agent-team', topology: 'fanout', role: 'reviewer' },
  }),
  middleware([trace()]),
  agent(agentCommand),
).as(reviewerBaseName)

const coordinator = compose(
  sandbox({
    resources: [workspace],
    labels: { demo: 'multi-agent-team', topology: 'peer', role: 'coordinator' },
  }),
  middleware([trace(), peerMiddleware({ peers: ['approver'] })]),
  agent(agentCommand),
).as('coordinator')

const approver = compose(
  sandbox({
    resources: [workspace],
    labels: { demo: 'multi-agent-team', topology: 'peer', role: 'approver' },
  }),
  middleware([trace(), peerMiddleware({ peers: ['coordinator'] })]),
  agent(agentCommand),
).as('approver')

const stageHandles = await pipe(researcher, writer).start({
  serverUrl,
  stateStream: sharedStateStream,
})
const reviewerHandles = await fanout(reviewer, { count: reviewerCount }).start({
  serverUrl,
  stateStream: sharedStateStream,
  name: reviewerBaseName,
})
const reviewerRuntimeNames = Array.from(
  { length: reviewerCount },
  (_, index) => `${reviewerBaseName}-${index + 1}`,
)
const specialistHandles = await peer(coordinator, approver).start({
  serverUrl,
  stateStream: sharedStateStream,
})
const teamNames = [
  ...Object.keys(stageHandles),
  ...reviewerRuntimeNames,
  ...Object.keys(specialistHandles),
]

const allHandles = [
  stageHandles.researcher,
  stageHandles.writer,
  ...reviewerHandles,
  specialistHandles.coordinator,
  specialistHandles.approver,
]

const db = await fireline.db({ stateStreamUrl: stageHandles.researcher.state.url })
const researcherRun = await openSession(stageHandles.researcher, 'researcher')
const writerRun = await openSession(stageHandles.writer, 'writer')
const reviewerRuns = await Promise.all(
  reviewerHandles.map((handle, index) => openSession(handle, `reviewer-${index + 1}`)),
)
const coordinatorRun = await openSession(specialistHandles.coordinator, 'coordinator')
const approverRun = await openSession(specialistHandles.approver, 'approver')

try {
  const researchPrompt = teamPrompt(
    'Map the three biggest launch risks in this repository, explain why they matter, and keep the answer concise enough to hand to another agent.',
  )
  const researchBrief = await promptAndReadText(
    researcherRun.acp,
    db,
    researcherRun.sessionId,
    researchPrompt,
  )

  const reviewerPrompts = [
    `Review this research from the product-risk lens and call out what would block a launch.

${researchBrief}`,
    `Review this research from the reliability lens and call out where the system could fail under load or restart pressure.

${researchBrief}`,
    `Review this research from the operator lens and call out what a support or on-call team would need to see live.

${researchBrief}`,
  ]
  const reviewerNotes = await Promise.all(
    reviewerRuns.map((run, index) =>
      promptAndReadText(
        run.acp,
        db,
        run.sessionId,
        teamPrompt(
          reviewerPrompts[index] ?? reviewerPrompts[reviewerPrompts.length - 1],
        ),
      ),
    ),
  )

  const coordinatorPrompt =
    commandMode === 'testy'
      ? firelinePeerTool('list_peers')
      : teamPrompt(
          `You are coordinating a Fireline team with these named specialists: ${Object.keys(specialistHandles).join(', ')}. Explain how named peers fit alongside staged and fanout workers in one deployment.`,
        )
  const coordinatorNote = await promptAndReadText(
    coordinatorRun.acp,
    db,
    coordinatorRun.sessionId,
    coordinatorPrompt,
  )

  const writerPrompt = teamPrompt(
    [
      'Turn this launch-team output into a one-page shipping brief.',
      '',
      'Research:',
      researchBrief,
      '',
      'Reviewer notes:',
      reviewerNotes
        .map((note, index) => `Reviewer ${index + 1}: ${note}`)
        .join('\n\n'),
      '',
      'Coordinator note:',
      coordinatorNote,
    ].join('\n'),
  )
  const finalBrief = await promptAndReadText(
    writerRun.acp,
    db,
    writerRun.sessionId,
    writerPrompt,
  )

  const approverPrompt = teamPrompt(
    `You are the named approver on this Fireline team. Read the shipping brief below and return a go/no-go recommendation with the one caveat that matters most.

${finalBrief}`,
  )
  const approvalDecision = await promptAndReadText(
    approverRun.acp,
    db,
    approverRun.sessionId,
    approverPrompt,
  )
  const peerDiscovery =
    commandMode === 'testy'
      ? extractKnownPeerNames(coordinatorNote, teamNames)
      : teamNames

  console.log(
    JSON.stringify(
      {
        question:
          'Can I compose staged handoff, parallel reviewers, and peer-ready specialists in one Fireline team?',
        stateStream: stageHandles.researcher.state.url,
        topologies: {
          pipe: Object.keys(stageHandles),
          fanout: reviewerRuntimeNames,
          peer: Object.keys(specialistHandles),
        },
        sessions: db.sessions.toArray.map((row) => ({
          sessionId: row.sessionId,
          state: row.state,
        })),
        promptRequests: db.promptRequests.toArray.length,
        peerDiscovery,
        coordinatorNote:
          commandMode === 'testy'
            ? 'Verified through fireline-peer.list_peers; see peerDiscovery.'
            : coordinatorNote,
        researchBrief,
        reviewerNotes,
        finalBrief,
        approvalDecision,
      },
      null,
      2,
    ),
  )
} finally {
  await Promise.allSettled([
    researcherRun.acp.close(),
    writerRun.acp.close(),
    ...reviewerRuns.map((run) => run.acp.close()),
    coordinatorRun.acp.close(),
    approverRun.acp.close(),
  ])
  db.close()
  await Promise.allSettled(allHandles.map((handle) => handle.stop()))
}

async function openSession(
  handle: FirelineAgent<string>,
  clientName: string,
): Promise<{ acp: ConnectedAcp; sessionId: SessionId }> {
  const acp = await handle.connect(`multi-agent-team:${clientName}`)
  const session = await acp.newSession({
    cwd: '/workspace',
    mcpServers: [],
  })
  return { acp, sessionId: session.sessionId }
}

async function promptAndReadText(
  acp: ConnectedAcp,
  db: FirelineDB,
  sessionId: SessionId,
  promptText: string,
  timeoutMs = 20_000,
): Promise<string> {
  await acp.prompt({
    sessionId,
    prompt: [{ type: 'text', text: promptText }],
  })

  const request = await waitForPromptRequest(db, sessionId, promptText, timeoutMs)
  return waitForRequestText(db, sessionId, request.requestId, timeoutMs)
}

function waitForPromptRequest(
  db: FirelineDB,
  sessionId: SessionId,
  promptText: string,
  timeoutMs: number,
): Promise<PromptRequestRow> {
  return new Promise((resolve, reject) => {
    let settled = false
    let timer: ReturnType<typeof setTimeout> | undefined
    let subscription: { unsubscribe(): void } | undefined

    const cleanup = () => {
      if (timer) {
        clearTimeout(timer)
      }
      subscription?.unsubscribe()
    }

    const finish = (value: PromptRequestRow) => {
      if (settled) {
        return
      }
      settled = true
      cleanup()
      resolve(value)
    }

    const fail = () => {
      if (settled) {
        return
      }
      settled = true
      cleanup()
      reject(
        new Error(`timed out waiting for completed prompt request: ${promptText}`),
      )
    }

    const publish = (rows: PromptRequestRow[]) => {
      const match = rows.find(
        (row) =>
          row.sessionId === sessionId &&
          row.text === promptText &&
          row.state === 'completed',
      )
      if (match) {
        finish(match)
      }
    }

    subscription = db.promptRequests.subscribe((rows) => publish(rows))
    timer = setTimeout(fail, timeoutMs)
    publish(db.promptRequests.toArray)
  })
}

function waitForRequestText(
  db: FirelineDB,
  sessionId: SessionId,
  requestId: RequestId,
  timeoutMs: number,
): Promise<string> {
  return new Promise((resolve, reject) => {
    let settled = false
    let timer: ReturnType<typeof setTimeout> | undefined
    let promptSubscription: { unsubscribe(): void } | undefined
    let chunkSubscription: { unsubscribe(): void } | undefined

    const cleanup = () => {
      if (timer) {
        clearTimeout(timer)
      }
      promptSubscription?.unsubscribe()
      chunkSubscription?.unsubscribe()
    }

    const finish = (value: string) => {
      if (settled) {
        return
      }
      settled = true
      cleanup()
      resolve(value)
    }

    const fail = () => {
      if (settled) {
        return
      }
      settled = true
      cleanup()
      reject(
        new Error(
          `timed out waiting for chunk text for ${String(sessionId)}:${String(requestId)}`,
        ),
      )
    }

    const publish = () => {
      const request = db.promptRequests.toArray.find(
        (row) => row.sessionId === sessionId && row.requestId === requestId,
      )
      if (!request || request.state !== 'completed') {
        return
      }

      const text = db.chunks.toArray
        .filter((row) => row.sessionId === sessionId && row.requestId === requestId)
        .map((row) => extractChunkTextPreview(row.update))
        .join('')
        .trim()

      if (text.length > 0) {
        finish(text)
      }
    }

    promptSubscription = db.promptRequests.subscribe(() => publish())
    chunkSubscription = db.chunks.subscribe(() => publish())
    timer = setTimeout(fail, timeoutMs)
    publish()
  })
}

function teamPrompt(text: string): string {
  return commandMode === 'testy'
    ? JSON.stringify({ command: 'echo', message: text })
    : text
}

function firelinePeerTool(
  tool: string,
  params: Record<string, unknown> = {},
): string {
  return JSON.stringify({
    command: 'call_tool',
    server: 'fireline-peer',
    tool,
    params,
  })
}

function splitCommand(command: string): string[] {
  return command.split(' ').filter((entry) => entry.length > 0)
}

function extractKnownPeerNames(rawText: string, names: string[]): string[] {
  return names.filter((name) => rawText.includes(name))
}
