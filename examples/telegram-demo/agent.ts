import { fileURLToPath } from 'node:url'
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, telegram, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

const repoPath = process.env.REPO_PATH ?? '../..'
const allowedUserIds = splitCsv(process.env.TELEGRAM_ALLOWED_USER_IDS)
const agentCommand = splitCommand(
  process.env.AGENT_COMMAND ?? 'npx -y @agentclientprotocol/claude-agent-acp',
)

const spec = compose(
  sandbox({
    resources: [localPath(repoPath, '/workspace')],
    labels: { demo: 'telegram-demo', channel: 'telegram' },
  }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' as const }),
    telegram({
      token: { ref: 'env:TELEGRAM_BOT_TOKEN' },
      chatId: process.env.TELEGRAM_CHAT_ID,
      allowedUserIds: allowedUserIds.length > 0 ? allowedUserIds : undefined,
      scope: 'tool_calls',
    }),
  ]),
  agent(agentCommand),
)

export default spec

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const handle = await spec.start({
    serverUrl: process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440',
    name: 'telegram-demo',
  })
  console.log(
    JSON.stringify(
      {
        demo: 'telegram-demo',
        transport: 'telegram',
        approvalScope: 'tool_calls',
        acp: handle.acp.url,
        stateStream: handle.state.url,
        chatId: process.env.TELEGRAM_CHAT_ID ?? null,
        allowedUserIds,
      },
      null,
      2,
    ),
  )
}

function splitCommand(command: string): string[] {
  return command.split(' ').filter((entry) => entry.length > 0)
}

function splitCsv(value: string | undefined): string[] {
  if (!value) {
    return []
  }
  return value
    .split(',')
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0)
}
