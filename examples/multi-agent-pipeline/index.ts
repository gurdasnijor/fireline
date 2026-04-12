// Fireline
import { agent, compose, middleware, pipe, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'
import { createFirelineDB } from '@fireline/state'

// Third-party

// User code
import { openNodeAcpConnection } from '../shared/acp-node.js'

const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const agentCommand = (
  process.env.AGENT_COMMAND ?? '../../target/debug/fireline-testy-prompt'
).split(' ')
const workspace = localPath(process.env.WORKSPACE_PATH ?? process.cwd(), '/workspace', true)

const stage = (name: string) =>
  compose(
    sandbox({ resources: [workspace], labels: { demo: 'multi-agent-pipeline', stage: name } }),
    middleware([trace({ includeMethods: ['session/prompt'] })]),
    agent(agentCommand),
  ).as(name)

const handles = await pipe(stage('researcher'), stage('reviewer'), stage('writer')).start({
  serverUrl,
  name: 'multi-agent-pipeline',
})

const db = createFirelineDB({ stateStreamUrl: handles.researcher.state.url })
await db.preload()
const researcher = await openNodeAcpConnection(handles.researcher.acp.url, 'pipeline-researcher')
const reviewer = await openNodeAcpConnection(handles.reviewer.acp.url, 'pipeline-reviewer')
const writer = await openNodeAcpConnection(handles.writer.acp.url, 'pipeline-writer')
const research = await researcher.connection.newSession({ cwd: '/workspace', mcpServers: [] })
const review = await reviewer.connection.newSession({ cwd: '/workspace', mcpServers: [] })
const write = await writer.connection.newSession({ cwd: '/workspace', mcpServers: [] })

await researcher.connection.prompt({ sessionId: research.sessionId, prompt: [{ type: 'text', text: 'Research: why durable streams make handoff safe.' }] })
const researchText = await waitForOutput(db, research.sessionId)
await reviewer.connection.prompt({ sessionId: review.sessionId, prompt: [{ type: 'text', text: `Review and tighten:\n${researchText}` }] })
const reviewText = await waitForOutput(db, review.sessionId)
await writer.connection.prompt({ sessionId: write.sessionId, prompt: [{ type: 'text', text: `Write the final demo script:\n${reviewText}` }] })
const finalText = await waitForOutput(db, write.sessionId)

console.log(
  JSON.stringify(
    {
      message: 'the agents do not coordinate directly; the stream does',
      sessions: db.collections.sessions.toArray.map((row) => ({ sessionId: row.sessionId, runtimeKey: row.runtimeKey })),
      promptTurns: db.collections.promptTurns.toArray.map((row) => ({ sessionId: row.sessionId, text: row.text, state: row.state })),
      finalText,
    },
    null,
    2,
  ),
)

await researcher.close()
await reviewer.close()
await writer.close()
db.close()

async function waitForOutput(db: ReturnType<typeof createFirelineDB>, sessionId: string) {
  return new Promise<string>((resolve, reject) => {
    let turns = db.collections.promptTurns.toArray
    let chunks = db.collections.chunks.toArray
    const timeout = setTimeout(() => {
      turnSub.unsubscribe()
      chunkSub.unsubscribe()
      reject(new Error(`timed out waiting for output for session ${sessionId}`))
    }, 5_000)
    const maybeResolve = () => {
      const completed = turns.find((row) => row.sessionId === sessionId && row.state === 'completed')
      if (!completed) return
      const text = chunks
        .filter((row) => row.promptTurnId === completed.promptTurnId)
        .map((row) => row.content)
        .join('')
      if (!text) return
      clearTimeout(timeout)
      turnSub.unsubscribe()
      chunkSub.unsubscribe()
      resolve(text)
    }
    const turnSub = db.collections.promptTurns.subscribeChanges(() => {
      turns = db.collections.promptTurns.toArray
      maybeResolve()
    })
    const chunkSub = db.collections.chunks.subscribeChanges(() => {
      chunks = db.collections.chunks.toArray
      maybeResolve()
    })
    maybeResolve()
  })
}
