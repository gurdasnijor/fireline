import fireline, { agent, compose, middleware, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'

const primaryUrl = process.env.FIRELINE_PRIMARY_URL ?? 'http://127.0.0.1:4440'
const rescueUrl = process.env.FIRELINE_RESCUE_URL ?? 'http://127.0.0.1:5440'
const stateStream = process.env.STATE_STREAM ?? `crash-proof-${Date.now()}`
const harness = compose(sandbox({ labels: { demo: 'crash-proof-agent' } }), middleware([trace({ includeMethods: ['session/new', 'session/load', 'session/prompt'] })]), agent((process.env.AGENT_COMMAND ?? '../../target/debug/fireline-testy-load').split(' ')))
const first = await harness.start({ serverUrl: primaryUrl, name: 'crash-proof-primary', stateStream })
const acp1 = await first.connect('crash-proof-primary')
const { sessionId } = await acp1.newSession({ cwd: '/workspace', mcpServers: [] })
await acp1.prompt({ sessionId, prompt: [{ type: 'text', text: 'Turn one: start auditing the repo and keep going after a crash.' }] })
await first.stop(); await acp1.close()
const second = await harness.start({ serverUrl: rescueUrl, name: 'crash-proof-rescue', stateStream })
const db = await fireline.db({ stateStreamUrl: second.state.url })
const acp2 = await second.connect('crash-proof-rescue')
await acp2.loadSession({ sessionId, cwd: '/workspace', mcpServers: [] })
await acp2.prompt({ sessionId, prompt: [{ type: 'text', text: 'Turn two: finish the audit without repeating yourself.' }] })
const turns = await waitForCompletedTurns(db.promptTurns, sessionId, 2, 10_000)
console.log(JSON.stringify({ question: 'What happens when the agent crashes mid-task?', sessionId, firstSandboxId: first.id, secondSandboxId: second.id, turns: turns.filter((row) => row.sessionId === sessionId).map((row) => row.text) }, null, 2))
await acp2.close(); db.close()

function waitForCompletedTurns(
  collection: {
    readonly toArray: readonly {
      readonly sessionId: string
      readonly state: string
      readonly text?: string
    }[]
    subscribe(callback: (rows: readonly {
      readonly sessionId: string
      readonly state: string
      readonly text?: string
    }[]) => void): { unsubscribe(): void }
  },
  sessionId: string,
  count: number,
  timeoutMs: number,
) {
  return new Promise<readonly {
    readonly sessionId: string
    readonly state: string
    readonly text?: string
  }[]>((resolve, reject) => {
    const timeout = setTimeout(() => {
      subscription.unsubscribe()
      reject(new Error(`timed out after ${timeoutMs}ms`))
    }, timeoutMs)
    const check = (rows: readonly {
      readonly sessionId: string
      readonly state: string
      readonly text?: string
    }[]) => rows.filter((row) => row.sessionId === sessionId && row.state === 'completed').length >= count
    const subscription = collection.subscribe((rows) => {
      if (!check(rows)) return
      clearTimeout(timeout)
      subscription.unsubscribe()
      resolve([...rows])
    })
  })
}
