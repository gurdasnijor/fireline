import { Sandbox, agent, compose, middleware, sandbox } from '../../dist/index.js'
import {
  autoApprove,
  peerRouting,
  telegram,
  wakeDeployment,
  webhook,
} from '../../dist/middleware.js'

const webhookUrl =
  process.env.WEBHOOK_URL ?? 'http://127.0.0.1:9999/hooks/durable-subscriber'

let capturedBody = null

globalThis.fetch = async (_url, request) => {
  capturedBody = JSON.parse(String(request?.body ?? 'null'))
  return new Response(
    JSON.stringify({
      id: 'sandbox-1',
      provider: 'local',
      acp: { url: 'ws://127.0.0.1:9000' },
      state: { url: 'http://127.0.0.1:7474/v1/stream/state' },
    }),
    {
      status: 200,
      headers: { 'content-type': 'application/json' },
    },
  )
}

const harness = compose(
  sandbox({ provider: 'local' }),
  middleware([
    autoApprove(),
    webhook({
      target: 'slack-approvals',
      url: webhookUrl,
      events: ['permission_request'],
      keyBy: 'session_request',
    }),
    telegram({
      token: 'test-bot-token',
      chatId: 'chat-42',
      events: ['permission_request'],
      keyBy: 'session_request',
    }),
    peerRouting(),
    wakeDeployment(),
  ]),
  agent(['node', 'agent.mjs']),
).as('durable-subscriber-integration')

await new Sandbox({ serverUrl: 'http://127.0.0.1:4440' }).provision(harness)

if (!capturedBody?.topology) {
  throw new Error('failed to capture lowered topology from Sandbox.provision')
}

process.stdout.write(`${JSON.stringify(capturedBody.topology)}\n`)
