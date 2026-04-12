// Fireline
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { contextInjection, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'
import { createFirelineDB } from '@fireline/state'

// Third-party
import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

// User code
import { openNodeAcpConnection } from '../shared/acp-node.js'

const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const workspace = await mkdtemp(join(tmpdir(), 'fireline-context-'))
await writeFile(join(workspace, 'RULES.md'), 'Always mention the codename Project Comet.\n')
await writeFile(join(workspace, 'CONTEXT.md'), 'Current milestone: keynote proxy-chain demo.\n')

const handle = await compose(
  sandbox({ resources: [localPath(workspace, '/workspace', true)], labels: { demo: 'context-injection' } }),
  middleware([
    trace({ includeMethods: ['session/prompt'] }),
    contextInjection({
      sources: [
        { kind: 'workspaceFile', path: '/workspace/RULES.md' },
        { kind: 'workspaceFile', path: '/workspace/CONTEXT.md' },
      ],
    }),
  ]),
  agent((process.env.AGENT_COMMAND ?? '../../target/debug/fireline-testy-prompt').split(' ')),
).start({ serverUrl, name: 'context-injection' })

const db = createFirelineDB({ stateStreamUrl: handle.state.url })
await db.preload()
const acp = await openNodeAcpConnection(handle.acp.url, 'context-injection')
const { sessionId } = await acp.connection.newSession({ cwd: '/workspace', mcpServers: [] })
const result = waitForContextResult(db, sessionId)
await acp.connection.prompt({ sessionId, prompt: [{ type: 'text', text: 'What is the codename?' }] })
const { rawPrompt, agentVisiblePrompt } = await result

console.log(
  JSON.stringify(
    {
      message: 'the proxy chain changed the prompt before the agent saw it',
      rawPrompt,
      agentVisiblePrompt,
    },
    null,
    2,
  ),
)

await acp.close()
db.close()

function waitForContextResult(db: ReturnType<typeof createFirelineDB>, sessionId: string) {
  return new Promise<{ rawPrompt: string | null; agentVisiblePrompt: string }>((resolve, reject) => {
    let turns = db.collections.promptTurns.toArray
    let chunks = db.collections.chunks.toArray
    const timeout = setTimeout(() => {
      turnSub.unsubscribe()
      chunkSub.unsubscribe()
      reject(new Error('timed out waiting for context-injection output'))
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
      resolve({ rawPrompt: completed.text ?? null, agentVisiblePrompt: text })
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
