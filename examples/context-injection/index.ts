// Fireline
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { contextInjection, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'
import { createFirelineDB } from '@fireline/state'

// User code
import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { openNodeAcpConnection } from '../shared/acp-node.js'
import { waitForRows } from '../shared/state-subscribe.js'

const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const workspace = await mkdtemp(join(tmpdir(), 'fireline-context-'))
await writeFile(join(workspace, 'RULES.md'), 'Always mention the codename Project Comet.\n')
await writeFile(join(workspace, 'CONTEXT.md'), 'Current milestone: keynote proxy-chain demo.\n')

const handle = await compose(
  sandbox({ resources: [localPath(workspace, '/workspace', true)], labels: { demo: 'context-injection' } }),
  middleware([
    trace({ includeMethods: ['session/prompt'] }),
    contextInjection({ files: ['/workspace/RULES.md', '/workspace/CONTEXT.md'] }),
  ]),
  agent((process.env.AGENT_COMMAND ?? '../../target/debug/fireline-testy-prompt').split(' ')),
).start({ serverUrl, name: 'context-injection' })

const db = createFirelineDB({ stateStreamUrl: handle.state.url })
await db.preload()
const acp = await openNodeAcpConnection(import.meta.url, handle.acp.url, 'context-injection')
const { sessionId } = await acp.connection.newSession({ cwd: '/workspace', mcpServers: [] })
await acp.connection.prompt({ sessionId, prompt: [{ type: 'text', text: 'What is the codename?' }] })

const turns = await waitForRows(
  db.collections.promptTurns,
  (rows) => rows.some((row) => row.sessionId === sessionId && row.state === 'completed'),
  5_000,
)
const chunks = await waitForRows(
  db.collections.chunks,
  (rows) => rows.some((row) => turns.some((turn) => turn.promptTurnId === row.promptTurnId)),
  5_000,
)

console.log(
  JSON.stringify(
    {
      message: 'the proxy chain changed the prompt before the agent saw it',
      rawPrompt: turns.find((row) => row.sessionId === sessionId)?.text,
      agentVisiblePrompt: chunks
        .filter((row) => turns.some((turn) => turn.promptTurnId === row.promptTurnId))
        .map((row) => row.content)
        .join(''),
    },
    null,
    2,
  ),
)

await acp.close()
db.close()
