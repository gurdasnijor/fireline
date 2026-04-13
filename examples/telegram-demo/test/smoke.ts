import assert from 'node:assert/strict'
import spec from '../agent.ts'

const token = process.env.TELEGRAM_BOT_TOKEN
if (!token) throw new Error('TELEGRAM_BOT_TOKEN is required')

const response = await fetch(`https://api.telegram.org/bot${token}/getMe`)
const payload = (await response.json()) as {
  ok?: boolean
  result?: { username?: string }
}

assert.equal(payload.ok, true, 'Telegram getMe must succeed')

const middlewareKinds = spec.middleware.chain.map((entry) => entry.kind)
assert.deepEqual(middlewareKinds, ['trace', 'approve', 'telegram'])

console.log(JSON.stringify({ username: payload.result?.username ?? null, middlewareKinds }, null, 2))
