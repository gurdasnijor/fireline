import assert from 'node:assert/strict'
import {
  execFileSync,
  spawn,
} from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { createServer } from 'node:net'
import path from 'node:path'
import readline from 'node:readline'
import test, { after, before } from 'node:test'
import { fileURLToPath } from 'node:url'

import { DurableStream } from '@durable-streams/client'

const here = path.dirname(fileURLToPath(import.meta.url))
const exampleDir = path.resolve(here, '..')
const repoRoot = path.resolve(exampleDir, '../..')
const cargoTargetDir = path.resolve(repoRoot, process.env.CARGO_TARGET_DIR ?? 'target')
const firelineStreamsBin = path.join(cargoTargetDir, 'debug', 'fireline-streams')
const pnpmBin = process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm'

let streamsPort = 0
let streamsProcess

before(async () => {
  execFileSync('cargo', ['build', '--quiet', '--bin', 'fireline-streams'], {
    cwd: repoRoot,
    stdio: 'inherit',
    env: process.env,
  })

  streamsPort = await reservePort()
  streamsProcess = spawn(firelineStreamsBin, ['--port', String(streamsPort)], {
    cwd: repoRoot,
    stdio: ['ignore', 'ignore', 'inherit'],
  })
  await waitForHttpOk(`http://127.0.0.1:${streamsPort}/healthz`, 'fireline-streams')
})

after(async () => {
  await stopChild(streamsProcess)
})

test(
  'wait resumes after another process resolves the same session-scoped awakeable',
  async () => {
    const streamUrl = await createJsonStateStream(`temporal-agent-${randomUUID()}`)
    const sessionId = `session-${randomUUID()}`
    const resolution = {
      note: 'Nightly window is open. Resume the rollout.',
      openedBy: 'release-manager',
      window: 'tonight-23:00',
    }

    const waiter = startExampleProcess('wait', {
      SESSION_ID: sessionId,
      STATE_STREAM_URL: streamUrl,
    })

    const waiting = await waiter.read(
      (message) => message.status === 'waiting',
      'waiter did not publish the waiting record',
    )
    assert.equal(waiting.sessionId, sessionId)
    assert.equal(waiting.windowKey?.kind, 'session')
    assert.equal(waiting.windowKey?.sessionId, sessionId)

    const resolver = startExampleProcess('resolve', {
      CHANGE_WINDOW: resolution.window,
      OPENED_BY: resolution.openedBy,
      RESOLUTION_NOTE: resolution.note,
      SESSION_ID: sessionId,
      STATE_STREAM_URL: streamUrl,
    })

    const resolved = await resolver.read(
      (message) => message.status === 'resolved',
      'resolver did not publish the resolved record',
    )
    assert.deepEqual(resolved.resolution, resolution)
    await resolver.waitForExit()

    const resumed = await waiter.read(
      (message) => message.status === 'resumed',
      'waiter did not resume after the durable completion',
    )
    assert.deepEqual(resumed.resolution, resolution)
    await waiter.waitForExit()

    const rows = await readRows(streamUrl)
    assert.equal(countRows(rows, `session:${sessionId}:waiting`, 'awakeable_waiting'), 1)
    assert.equal(countRows(rows, `session:${sessionId}:resolved`, 'awakeable_resolved'), 1)
  },
  { timeout: 30_000 },
)

function startExampleProcess(command, env) {
  const child = spawn(
    pnpmBin,
    ['exec', 'tsx', '--tsconfig', './tsconfig.json', 'index.ts', command],
    {
      cwd: exampleDir,
      env: {
        ...process.env,
        ...env,
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  )

  const queue = []
  const waiters = []
  const stderr = []

  readline.createInterface({
    input: child.stdout,
    crlfDelay: Infinity,
  }).on('line', (line) => {
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

  readline.createInterface({
    input: child.stderr,
    crlfDelay: Infinity,
  }).on('line', (line) => {
    if (line.trim()) {
      stderr.push(line)
    }
  })

  const waitForExit = async () => {
    if (child.exitCode !== null) {
      assert.equal(
        child.exitCode,
        0,
        `example process exited with code ${String(child.exitCode)}\n${stderr.join('\n')}`,
      )
      return
    }

    const code = await new Promise((resolve) => {
      child.once('exit', resolve)
    })
    assert.equal(
      code,
      0,
      `example process exited with code ${String(code)}\n${stderr.join('\n')}`,
    )
  }

  return {
    read(predicate, failureLabel, timeoutMs = 10_000) {
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
          reject(new Error(`${failureLabel}\n${stderr.join('\n')}`))
        }, timeoutMs)
        waiters.push({ predicate, resolve, reject, timeout })
      })
    },
    waitForExit,
  }
}

async function createJsonStateStream(label) {
  const streamUrl = `http://127.0.0.1:${streamsPort}/v1/stream/${label}`
  const stream = new DurableStream({
    url: streamUrl,
    contentType: 'application/json',
  })
  await stream.create({
    contentType: 'application/json',
  })
  return streamUrl
}

async function readRows(streamUrl) {
  const url = new URL(streamUrl)
  url.searchParams.set('offset', '-1')
  const response = await fetch(url)
  if (!response.ok) {
    throw new Error(`failed to read stream ${streamUrl}: ${response.status} ${response.statusText}`)
  }
  return await response.json()
}

function countRows(rows, key, kind) {
  return rows.filter(
    (row) => row.type === 'awakeable' && row.key === key && row.value?.kind === kind,
  ).length
}

async function reservePort() {
  return await new Promise((resolvePort, reject) => {
    const server = createServer()
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      if (!address || typeof address === 'string') {
        reject(new Error('failed to reserve local port'))
        return
      }
      server.close((error) => {
        if (error) {
          reject(error)
          return
        }
        resolvePort(address.port)
      })
    })
  })
}

async function waitForHttpOk(url, label, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url)
      if (response.ok) {
        return
      }
    } catch {
      // keep polling
    }
    await sleep(100)
  }
  throw new Error(`timed out waiting for ${label} at ${url}`)
}

async function stopChild(
  child,
) {
  if (!child || child.exitCode !== null || child.signalCode !== null) {
    return
  }
  child.kill('SIGTERM')
  await Promise.race([
    new Promise((resolve) => {
      child.once('exit', () => resolve())
    }),
    sleep(5_000),
  ])
}

function sleep(ms) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}
