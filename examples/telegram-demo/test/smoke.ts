import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import spec from '../agent.ts'

const middlewareKinds = spec.middleware.chain.map((entry) => entry.kind)
assert.deepEqual(middlewareKinds, ['trace', 'approve', 'telegram'])

const telegramMiddleware = spec.middleware.chain.find(
  (entry) => entry.kind === 'telegram',
)
assert.ok(telegramMiddleware, 'telegram middleware must be present')
if (telegramMiddleware?.kind !== 'telegram') {
  throw new Error('telegram middleware is missing')
}

assert.deepEqual(telegramMiddleware.token, { ref: 'env:TELEGRAM_BOT_TOKEN' })
const agentSource = await readFile(new URL('../agent.ts', import.meta.url), 'utf8')
assert.match(agentSource, /scope:\s*'tool_calls'/)

const token = process.env.TELEGRAM_BOT_TOKEN
let username: string | null = null

if (token) {
  const response = await fetch(`https://api.telegram.org/bot${token}/getMe`)
  const payload = (await response.json()) as {
    ok?: boolean
    result?: { username?: string }
  }
  assert.equal(payload.ok, true, 'Telegram getMe must succeed')
  username = payload.result?.username ?? null
}

console.log(
  JSON.stringify(
    {
      middlewareKinds,
      username,
      verifiedToken: Boolean(token),
    },
    null,
    2,
  ),
)
