import test from 'node:test'
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import path from 'node:path'
import readline from 'node:readline'

const here = path.dirname(fileURLToPath(import.meta.url))
const exampleDir = path.resolve(here, '..')

test('echo prompt returns an agent chunk and end_turn', async () => {
  const harness = await startHarness()
  try {
    await initialize(harness, {})
    const sessionId = await newSession(harness)

    harness.send({
      jsonrpc: '2.0',
      id: 3,
      method: 'session/prompt',
      params: {
        sessionId,
        prompt: [{ type: 'text', text: 'hello temporal agent' }],
      },
    })

    const update = await harness.read((message) => message.method === 'session/update')
    assert.equal(update.params.sessionId, sessionId)
    assert.equal(update.params.update.content.text, 'hello temporal agent')

    const result = await harness.read((message) => message.id === 3)
    assert.equal(result.result.stopReason, 'end_turn')
  } finally {
    await harness.close()
  }
})

test('wait prompt emits session/wait and completes when the client responds', async () => {
  const harness = await startHarness()
  try {
    await initialize(harness, {
      methods: ['session/wait'],
    })
    const sessionId = await newSession(harness)

    harness.send({
      jsonrpc: '2.0',
      id: 4,
      method: 'session/prompt',
      params: {
        sessionId,
        prompt: [{ type: 'text', text: 'wait 5s' }],
      },
    })

    const outgoing = await harness.read((message) => message.method === 'session/wait')
    assert.equal(outgoing.params.sessionId, sessionId)
    assert.equal(outgoing.params.ms, 5000)

    harness.send({
      jsonrpc: '2.0',
      id: outgoing.id,
      result: { ok: true },
    })

    const update = await harness.read((message) => message.method === 'session/update')
    assert.equal(update.params.update.content.text, 'waited 5 seconds')

    const result = await harness.read((message) => message.id === 4)
    assert.equal(result.result.stopReason, 'end_turn')
  } finally {
    await harness.close()
  }
})

test('schedule prompt emits session/schedule', async () => {
  const harness = await startHarness()
  try {
    await initialize(harness, {
      methods: ['session/schedule'],
    })
    const sessionId = await newSession(harness)

    harness.send({
      jsonrpc: '2.0',
      id: 5,
      method: 'session/prompt',
      params: {
        sessionId,
        prompt: [{ type: 'text', text: 'schedule hello in 10s' }],
      },
    })

    const outgoing = await harness.read((message) => message.method === 'session/schedule')
    assert.equal(outgoing.params.sessionId, sessionId)
    assert.equal(outgoing.params.delayMs, 10000)
    assert.deepEqual(outgoing.params.prompt, [{ type: 'text', text: 'hello' }])

    harness.send({
      jsonrpc: '2.0',
      id: outgoing.id,
      result: { scheduled: true },
    })

    const update = await harness.read((message) => message.method === 'session/update')
    assert.equal(update.params.update.content.text, 'scheduled "hello" in 10 seconds')

    const result = await harness.read((message) => message.id === 5)
    assert.equal(result.result.stopReason, 'end_turn')
  } finally {
    await harness.close()
  }
})

test('wait_for prompt emits session/wait_for', async () => {
  const harness = await startHarness()
  try {
    await initialize(harness, {
      methods: ['session/wait_for'],
    })
    const sessionId = await newSession(harness)

    harness.send({
      jsonrpc: '2.0',
      id: 6,
      method: 'session/prompt',
      params: {
        sessionId,
        prompt: [{ type: 'text', text: 'wait for event' }],
      },
    })

    const outgoing = await harness.read((message) => message.method === 'session/wait_for')
    assert.equal(outgoing.params.sessionId, sessionId)
    assert.deepEqual(outgoing.params.filter, { kind: 'event', name: 'demo.temporal' })

    harness.send({
      jsonrpc: '2.0',
      id: outgoing.id,
      result: { matched: true },
    })

    const update = await harness.read((message) => message.method === 'session/update')
    assert.equal(update.params.update.content.text, 'wait_for resolved for demo.temporal')

    const result = await harness.read((message) => message.id === 6)
    assert.equal(result.result.stopReason, 'end_turn')
  } finally {
    await harness.close()
  }
})

async function initialize(harness, temporal) {
  harness.send({
    jsonrpc: '2.0',
    id: 1,
    method: 'initialize',
    params: {
      protocolVersion: 1,
      clientCapabilities: {},
      serverCapabilities: {
        platform: temporal ? { temporal } : {},
      },
      clientInfo: {
        name: 'temporal-agent-smoke',
        version: '0.0.1',
      },
    },
  })
  const message = await harness.read((candidate) => candidate.id === 1)
  assert.equal(message.result.protocolVersion, 1)
}

async function newSession(harness) {
  harness.send({
    jsonrpc: '2.0',
    id: 2,
    method: 'session/new',
    params: {
      cwd: '/workspace',
      mcpServers: [],
    },
  })
  const message = await harness.read((candidate) => candidate.id === 2)
  assert.equal(typeof message.result.sessionId, 'string')
  return message.result.sessionId
}

async function startHarness() {
  const child = spawn(process.execPath, ['index.js'], {
    cwd: exampleDir,
    stdio: ['pipe', 'pipe', 'inherit'],
  })

  const queue = []
  const waiters = []
  const rl = readline.createInterface({ input: child.stdout, crlfDelay: Infinity })

  rl.on('line', (line) => {
    if (!line.trim()) {
      return
    }
    const message = JSON.parse(line)
    const waiterIndex = waiters.findIndex((waiter) => waiter.predicate(message))
    if (waiterIndex >= 0) {
      const [waiter] = waiters.splice(waiterIndex, 1)
      clearTimeout(waiter.timeout)
      waiter.resolve(message)
      return
    }
    queue.push(message)
  })

  return {
    send(message) {
      child.stdin.write(`${JSON.stringify(message)}\n`)
    },
    read(predicate, timeoutMs = 2000) {
      const queuedIndex = queue.findIndex(predicate)
      if (queuedIndex >= 0) {
        return Promise.resolve(queue.splice(queuedIndex, 1)[0])
      }

      return new Promise((resolve, reject) => {
        const timeout = setTimeout(() => {
          const index = waiters.findIndex((waiter) => waiter.timeout === timeout)
          if (index >= 0) {
            waiters.splice(index, 1)
          }
          reject(new Error('timed out waiting for agent message'))
        }, timeoutMs)

        waiters.push({ predicate, resolve, reject, timeout })
      })
    },
    async close() {
      child.kill('SIGTERM')
      await new Promise((resolve) => child.once('exit', () => resolve()))
      rl.close()
    },
  }
}
