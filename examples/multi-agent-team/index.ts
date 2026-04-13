import fireline, { agent, compose, middleware, pipe, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'
import { extractChunkTextPreview } from '@fireline/state'
import { waitForRows } from '../shared/wait.ts'

const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const command = (process.env.AGENT_COMMAND ?? '../../target/debug/fireline-testy-prompt').split(' ')
const workspace = localPath(process.env.WORKSPACE_PATH ?? process.cwd(), '/workspace', true)
const stage = (name: string) => compose(sandbox({ resources: [workspace], labels: { demo: 'multi-agent-team', stage: name } }), middleware([trace()]), agent(command)).as(name)
const handles = await pipe(stage('researcher'), stage('writer')).start({ serverUrl, name: 'multi-agent-team' })
const db = await fireline.db({ stateStreamUrl: handles.researcher.state.url })
const researcher = await handles.researcher.connect('team-researcher')
const writer = await handles.writer.connect('team-writer')
const research = await researcher.newSession({ cwd: '/workspace', mcpServers: [] })
await researcher.prompt({ sessionId: research.sessionId, prompt: [{ type: 'text', text: 'Find the three biggest risks in this repo.' }] })
const researchTurns = await waitForRows(
  db.promptRequests,
  (rows) => rows.some((row) => row.sessionId === research.sessionId && row.state === 'completed'),
  10_000,
)
const researchRequestIds = new Set(researchTurns.map((turn) => turn.requestId))
const researchText = db.chunks.toArray
  .filter((row) => researchRequestIds.has(row.requestId))
  .map((row) => extractChunkTextPreview(row.update))
  .join('')
const write = await writer.newSession({ cwd: '/workspace', mcpServers: [] })
await writer.prompt({ sessionId: write.sessionId, prompt: [{ type: 'text', text: `Turn this research into a one-page brief:\n${researchText}` }] })
const writerTurns = await waitForRows(
  db.promptRequests,
  (rows) => rows.some((row) => row.sessionId === write.sessionId && row.state === 'completed'),
  10_000,
)
const writerRequestIds = new Set(writerTurns.map((turn) => turn.requestId))
console.log(JSON.stringify({
  question: 'Can multiple agents collaborate on the same task?',
  sessions: db.sessions.toArray.map((row) => row.sessionId),
  graph: db.childSessionEdges.toArray,
  finalDocument: db.chunks.toArray
    .filter((row) => writerRequestIds.has(row.requestId))
    .map((row) => extractChunkTextPreview(row.update))
    .join(''),
}, null, 2))
await researcher.close(); await writer.close(); db.close()
