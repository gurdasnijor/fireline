import { fileURLToPath } from 'node:url'
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, telegram, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

const repoPath = process.env.REPO_PATH ?? '../..'
const agentCommand = (process.env.AGENT_COMMAND ?? 'pi-acp').split(' ')

const spec = compose(
  sandbox({ resources: [localPath(repoPath, '/workspace')], labels: { demo: 'telegram-demo' } }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' as const }),
    telegram({ token: { ref: 'env:TELEGRAM_BOT_TOKEN' }, events: ['permission_request'] as const }),
  ]),
  agent(agentCommand),
)

export default spec

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const handle = await spec.start({ serverUrl: process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440', name: 'telegram-demo' })
  console.log(JSON.stringify({ demo: 'telegram-demo', stateStream: handle.state.url }, null, 2))
}
