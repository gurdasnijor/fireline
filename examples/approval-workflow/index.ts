import fireline, {
  agent,
  compose,
  middleware,
  sandbox,
  type RequestId,
  type SessionId,
} from '@fireline/client'
import { approve, trace } from '@fireline/client/middleware'
import { createServer, type IncomingMessage } from 'node:http'

const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const webhookUrl = new URL(process.env.APPROVAL_WEBHOOK ?? 'http://127.0.0.1:8787/approval')
const agentCommand = (
  process.env.AGENT_COMMAND ?? 'npx -y @agentclientprotocol/claude-agent-acp'
).split(' ')
const usesTestAgent = agentCommand.some((part) => part.includes('fireline-testy-fs'))
const resolvedRequests = new Set<string>()
const approvals: Array<{
  sessionId: SessionId
  requestId: RequestId
  reason?: string
}> = []

const handle = await compose(
  sandbox({
    provider: 'local',
    fsBackend: 'streamFs',
    envVars: process.env.ANTHROPIC_API_KEY
      ? { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY }
      : undefined,
    labels: { demo: 'approval-workflow' },
  }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' as const }),
  ]),
  agent(agentCommand),
).start({ serverUrl, name: 'approval-workflow' })

const broker = createServer(async (req, res) => {
  if (req.method !== 'POST' || req.url !== webhookUrl.pathname) {
    res.writeHead(404).end('not found')
    return
  }

  const payload = JSON.parse(await readBody(req)) as WebhookDeliveryPayload
  const permission = readPermission(payload)
  if (!permission) {
    res.writeHead(202).end('ignored')
    return
  }

  const dedupeKey = `${permission.sessionId}:${String(permission.requestId)}`
  if (!resolvedRequests.has(dedupeKey)) {
    resolvedRequests.add(dedupeKey)
    approvals.push(permission)
    await handle.resolvePermission(permission.sessionId, permission.requestId, {
      allow: true,
      resolvedBy: 'approval-webhook-demo',
    })
  }

  res.writeHead(202).end('approved')
})

await new Promise<void>((resolve) =>
  broker.listen(
    {
      host: webhookUrl.hostname,
      port: Number(webhookUrl.port || '8787'),
    },
    resolve,
  ),
)

const db = await fireline.db({ stateStreamUrl: handle.state.url })
db.permissions.subscribe((rows) => {
  const pending = rows.find(
    (row) =>
      row.state === 'pending' &&
      !resolvedRequests.has(`${row.sessionId}:${String(row.requestId)}`),
  )
  if (!pending) return

  const body = JSON.stringify({
    sessionId: pending.sessionId,
    requestId: pending.requestId,
    reason: pending.title,
  })

  void fetch(webhookUrl, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body,
  })
})
const acp = await handle.connect('approval-workflow')

try {
  const { sessionId } = await acp.newSession({ cwd: '/workspace', mcpServers: [] })
  await acp.prompt({
    sessionId,
    prompt: [{ type: 'text', text: promptText() }],
  })

  const permissionRows = db.permissions.toArray
    .filter((row) => row.sessionId === sessionId)
    .map((row) => ({
      requestId: row.requestId,
      state: row.state,
      outcome: row.outcome,
      title: row.title,
    }))

  const agentText = db.chunks.toArray
    .filter((row) => row.sessionId === sessionId)
    .map((row) => chunkText(row.update))
    .filter((text) => text.length > 0)
    .at(-1)

  console.log(
    JSON.stringify(
      {
        question: 'How do I pause an agent, route the decision through a webhook, and resume the same run?',
        agentCommand,
        sessionId,
        stateStream: handle.state.url,
        webhook: webhookUrl.toString(),
        approvals,
        durablePermissions: permissionRows,
        latestAgentText: agentText,
      },
      null,
      2,
    ),
  )
} finally {
  await acp.close()
  db.close()
  await new Promise<void>((resolve) => broker.close(() => resolve()))
  await handle.destroy()
}

function promptText(): string {
  if (usesTestAgent) {
    return JSON.stringify({ command: 'ready' })
  }

  return process.env.APPROVAL_PROMPT ??
    'Write /workspace/approved-note.txt with a short note that says "approved after human review". Wait for approval before you make any file changes.'
}

function chunkText(update: unknown): string {
  if (!update || typeof update !== 'object') return ''
  const content = (update as { content?: { text?: unknown } }).content
  return typeof content?.text === 'string' ? content.text : ''
}

function readPermission(payload: WebhookDeliveryPayload): {
  sessionId: SessionId
  requestId: RequestId
  reason?: string
} | null {
  if (
    typeof payload.sessionId === 'string' &&
    (typeof payload.requestId === 'string' || typeof payload.requestId === 'number')
  ) {
    return {
      sessionId: payload.sessionId,
      requestId: payload.requestId,
      reason: payload.reason,
    }
  }

  const value = payload?.event?.value
  if (!value || value.kind !== 'permission_request') return null
  if (typeof value.sessionId !== 'string') return null
  if (typeof value.requestId !== 'string' && typeof value.requestId !== 'number') return null
  return {
    sessionId: value.sessionId,
    requestId: value.requestId,
    reason: value.reason,
  }
}

function readBody(req: IncomingMessage) {
  return new Promise<string>((resolve, reject) => {
    const chunks: Uint8Array[] = []
    req.on('data', (chunk: Uint8Array) => chunks.push(chunk))
    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')))
    req.on('error', reject)
  })
}

interface WebhookDeliveryPayload {
  readonly subscription?: string
  readonly offset?: string
  readonly sessionId?: SessionId
  readonly requestId?: RequestId
  readonly reason?: string
  readonly event?: {
    readonly value?: {
      readonly kind?: string
      readonly sessionId?: SessionId
      readonly requestId?: RequestId
      readonly reason?: string
    }
  }
}
