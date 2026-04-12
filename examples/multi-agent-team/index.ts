import fireline, { agent, compose, middleware, pipe, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

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
const researchTurns = await waitForCompletedTurns(db.promptTurns, research.sessionId, 10_000)
const researchText = db.chunks.toArray.filter((row) => researchTurns.some((turn) => turn.promptTurnId === row.promptTurnId)).map((row) => row.content).join('')
const write = await writer.newSession({ cwd: '/workspace', mcpServers: [] })
await writer.prompt({ sessionId: write.sessionId, prompt: [{ type: 'text', text: `Turn this research into a one-page brief:\n${researchText}` }] })
const writerTurns = await waitForCompletedTurns(db.promptTurns, write.sessionId, 10_000)
console.log(JSON.stringify({ question: 'Can multiple agents collaborate on the same task?', sessions: db.sessions.toArray.map((row) => row.sessionId), graph: db.childSessionEdges.toArray, finalDocument: db.chunks.toArray.filter((row) => writerTurns.some((turn) => turn.promptTurnId === row.promptTurnId)).map((row) => row.content).join('') }, null, 2))
await researcher.close(); await writer.close(); db.close()

function waitForCompletedTurns(
  collection: {
    readonly toArray: readonly {
      readonly sessionId: string
      readonly state: string
      readonly promptTurnId: string
    }[]
    subscribe(callback: (rows: readonly {
      readonly sessionId: string
      readonly state: string
      readonly promptTurnId: string
    }[]) => void): { unsubscribe(): void }
  },
  sessionId: string,
  timeoutMs: number,
) {
  return new Promise<readonly {
    readonly sessionId: string
    readonly state: string
    readonly promptTurnId: string
  }[]>((resolve, reject) => {
    const timeout = setTimeout(() => {
      subscription.unsubscribe()
      reject(new Error(`timed out after ${timeoutMs}ms`))
    }, timeoutMs)
    const subscription = collection.subscribe((rows) => {
      if (!rows.some((row) => row.sessionId === sessionId && row.state === 'completed')) return
      clearTimeout(timeout)
      subscription.unsubscribe()
      resolve([...rows])
    })
  })
}
