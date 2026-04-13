import fireline, { agent, compose, middleware, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'
import { extractChunkTextPreview } from '@fireline/state'

const question = 'Can I start a long-running agent task before lunch and check back later?'
const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const stateStreamUrl = process.env.TASK_STREAM_URL
const agentCommand = (
  process.env.AGENT_COMMAND ?? 'npx -y @agentclientprotocol/claude-agent-acp'
).split(' ')
const taskPrompt =
  process.env.TASK_PROMPT ??
  'Read this repository like you are preparing a next-morning handoff. Leave me a short summary of the three biggest risks and the first thing I should check when I get back.'

if (stateStreamUrl) {
  const db = await fireline.db({ stateStreamUrl })
  const latestAgentText = db.chunks.toArray
    .map((row) => extractChunkTextPreview(row.update))
    .filter((text) => text.length > 0)
    .at(-1)

  console.log(
    JSON.stringify(
      {
        question,
        stateStream: stateStreamUrl,
        sessions: db.sessions.toArray.map((row) => ({
          sessionId: row.sessionId,
          state: row.state,
          lastSeenAt: row.lastSeenAt,
        })),
        promptRequests: db.promptRequests.toArray.map((row) => ({
          sessionId: row.sessionId,
          requestId: row.requestId,
          state: row.state,
          text: row.text,
        })),
        latestAgentText,
      },
      null,
      2,
    ),
  )
  db.close()
  process.exit(0)
}

const handle = await compose(
  sandbox({
    provider: 'local',
    envVars: process.env.ANTHROPIC_API_KEY
      ? { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY }
      : undefined,
    labels: { demo: 'background-task' },
  }),
  middleware([trace()]),
  agent(agentCommand),
).start({ serverUrl, name: 'background-task' })

const acp = await handle.connect('background-task')

try {
  const { sessionId } = await acp.newSession({ cwd: '/workspace', mcpServers: [] })
  await acp.prompt({
    sessionId,
    prompt: [{ type: 'text', text: taskPrompt }],
  })

  console.log(
    JSON.stringify(
      {
        question,
        taskId: handle.id,
        sessionId,
        stateStream: handle.state.url,
        nextCheck: `TASK_STREAM_URL=${handle.state.url} pnpm start`,
      },
      null,
      2,
    ),
  )
} finally {
  await acp.close()
  await handle.destroy()
}
