import { agent, compose, connectAcp, middleware, pipe, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'
import { createFirelineDB } from '@fireline/state'
import { waitForRows } from '../shared/wait.js'

const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const command = (process.env.AGENT_COMMAND ?? '../../target/debug/fireline-testy-prompt').split(' ')
const workspace = localPath(process.env.WORKSPACE_PATH ?? process.cwd(), '/workspace', true)
const stage = (name: string) => compose(sandbox({ resources: [workspace], labels: { demo: 'multi-agent-team', stage: name } }), middleware([trace()]), agent(command)).as(name)
const handles = await pipe(stage('researcher'), stage('writer')).start({ serverUrl, name: 'multi-agent-team' })
const db = createFirelineDB({ stateStreamUrl: handles.researcher.state.url }); await db.preload()
const researcher = await connectAcp(handles.researcher.acp, 'team-researcher')
const writer = await connectAcp(handles.writer.acp, 'team-writer')
const research = await researcher.newSession({ cwd: '/workspace', mcpServers: [] })
await researcher.prompt({ sessionId: research.sessionId, prompt: [{ type: 'text', text: 'Find the three biggest risks in this repo.' }] })
const researchTurns = await waitForRows(db.collections.promptTurns, (rows) => rows.some((row) => row.sessionId === research.sessionId && row.state === 'completed'), 10_000)
const researchText = db.collections.chunks.toArray.filter((row) => researchTurns.some((turn) => turn.promptTurnId === row.promptTurnId)).map((row) => row.content).join('')
const write = await writer.newSession({ cwd: '/workspace', mcpServers: [] })
await writer.prompt({ sessionId: write.sessionId, prompt: [{ type: 'text', text: `Turn this research into a one-page brief:\n${researchText}` }] })
const writerTurns = await waitForRows(db.collections.promptTurns, (rows) => rows.some((row) => row.sessionId === write.sessionId && row.state === 'completed'), 10_000)
console.log(JSON.stringify({ question: 'Can multiple agents collaborate on the same task?', sessions: db.collections.sessions.toArray.map((row) => row.sessionId), graph: db.collections.childSessionEdges.toArray, finalDocument: db.collections.chunks.toArray.filter((row) => writerTurns.some((turn) => turn.promptTurnId === row.promptTurnId)).map((row) => row.content).join('') }, null, 2))
await researcher.close(); await writer.close(); db.close()
