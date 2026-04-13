import {
  execFileSync,
  spawn,
  type ChildProcessWithoutNullStreams,
} from 'node:child_process'
import { mkdtemp, rm } from 'node:fs/promises'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { randomUUID } from 'node:crypto'

import {
  afterAll,
  afterEach,
  beforeAll,
  describe,
  expect,
  it,
} from 'vitest'
import type { Stream } from '@agentclientprotocol/sdk'
import WebSocket, { type RawData } from 'ws'

import {
  agent,
  compose,
  middleware,
  sandbox,
  type ConnectedAcp,
  type FirelineAgent,
  type Harness,
} from '../src/index.js'

const repoRoot = fileURLToPath(new URL('../../../', import.meta.url))
const cargoTargetDir = resolve(repoRoot, process.env.CARGO_TARGET_DIR ?? 'target')
const firelineBin = join(cargoTargetDir, 'debug', 'fireline')
const firelineStreamsBin = join(cargoTargetDir, 'debug', 'fireline-streams')
const firelineTestyBin = join(cargoTargetDir, 'debug', 'fireline-testy')

let tempRoot: string
let streamsPort = 0
let controlPlanePort = 0
let streamsProcess: ChildProcessWithoutNullStreams | undefined
let controlPlaneProcess: ChildProcessWithoutNullStreams | undefined
let liveHandle: FirelineAgent<string> | undefined
let liveConnection: ConnectedAcp | undefined

describe('conductor.connect_to', () => {
  beforeAll(async () => {
    execFileSync(
      'cargo',
      ['build', '--quiet', '--bin', 'fireline', '--bin', 'fireline-streams', '--bin', 'fireline-testy'],
      {
        cwd: repoRoot,
        stdio: 'inherit',
        env: process.env,
      },
    )
    tempRoot = await mkdtemp(join(tmpdir(), 'fireline-client-conductor-'))
    streamsPort = await reservePort()
    controlPlanePort = await reservePort()

    streamsProcess = spawn(firelineStreamsBin, ['--port', String(streamsPort)], {
      cwd: repoRoot,
      stdio: 'inherit',
    })
    await waitForHttpOk(`http://127.0.0.1:${streamsPort}/healthz`, 'fireline-streams')

    controlPlaneProcess = spawn(
      firelineBin,
      [
        '--control-plane',
        '--host',
        '127.0.0.1',
        '--port',
        String(controlPlanePort),
        '--durable-streams-url',
        `http://127.0.0.1:${streamsPort}/v1/stream`,
      ],
      {
        cwd: repoRoot,
        stdio: 'inherit',
      },
    )
    await waitForHttpOk(`http://127.0.0.1:${controlPlanePort}/healthz`, 'fireline')
  }, 30_000)

  afterEach(async () => {
    await liveConnection?.close().catch(() => {})
    liveConnection = undefined
    await liveHandle?.stop().catch(() => {})
    liveHandle = undefined
  })

  afterAll(async () => {
    await liveConnection?.close().catch(() => {})
    await liveHandle?.stop().catch(() => {})
    await stopChild(controlPlaneProcess)
    await stopChild(streamsProcess)
    await rm(tempRoot, { recursive: true, force: true })
  })

  it('returns a conductor and keeps the deprecated Harness alias usable', () => {
    const conductor = createTestConductor().as('reviewer')
    const harness: Harness<'reviewer'> = conductor

    expect(conductor.kind).toBe('conductor')
    expect(conductor.role).toBe('client')
    expect(harness.name).toBe('reviewer')
    expect(typeof conductor.connect_to).toBe('function')
    expect(typeof harness.start).toBe('function')
  })

  it(
    'provisions hosted runtimes and returns a live ACP connection in one call',
    async () => {
      liveConnection = await createTestConductor().connect_to({
        kind: 'hosted',
        url: `http://127.0.0.1:${controlPlanePort}`,
        stateStream: `ts-conductor-hosted-${randomUUID()}`,
        clientName: 'conductor.hosted',
      })

      await expectEcho(liveConnection, 'hello from hosted conductor')
    },
    20_000,
  )

  it(
    'attaches over raw websocket transport',
    async () => {
      liveHandle = await createTestConductor().start({
        serverUrl: `http://127.0.0.1:${controlPlanePort}`,
        name: `ts-conductor-ws-${randomUUID()}`,
        stateStream: `ts-conductor-ws-${randomUUID()}`,
      })

      liveConnection = await createTestConductor().connect_to({
        kind: 'websocket',
        url: liveHandle.acp.url,
        headers: liveHandle.acp.headers,
        clientName: 'conductor.websocket',
      })

      await expectEcho(liveConnection, 'hello from websocket conductor')
    },
    20_000,
  )

  it(
    'attaches over a caller-supplied ACP stream',
    async () => {
      liveHandle = await createTestConductor().start({
        serverUrl: `http://127.0.0.1:${controlPlanePort}`,
        name: `ts-conductor-stream-${randomUUID()}`,
        stateStream: `ts-conductor-stream-${randomUUID()}`,
      })

      const stream = await openWebSocketStream(liveHandle.acp.url, liveHandle.acp.headers)
      liveConnection = await createTestConductor().connect_to({
        kind: 'stream',
        stream,
        clientName: 'conductor.stream',
      })

      await expectEcho(liveConnection, 'hello from stream conductor')
    },
    20_000,
  )

  it(
    'boots native stdio transport against fireline --acp-stdio',
    async () => {
      liveConnection = await createTestConductor().connect_to({
        kind: 'stdio',
        firelineBin,
        durableStreamsUrl: `http://127.0.0.1:${streamsPort}/v1/stream`,
        name: `ts-conductor-stdio-${randomUUID()}`,
        stateStream: `ts-conductor-stdio-${randomUUID()}`,
        cwd: repoRoot,
        clientName: 'conductor.stdio',
      })

      await expectEcho(liveConnection, 'hello from stdio conductor')
    },
    20_000,
  )
})

function createTestConductor() {
  return compose(
    sandbox({ provider: 'local' }),
    middleware([]),
    agent([firelineTestyBin]),
  )
}

async function expectEcho(acp: ConnectedAcp, message: string): Promise<void> {
  const session = await acp.newSession({
    cwd: repoRoot,
    mcpServers: [],
  })

  const response = await acp.prompt({
    sessionId: session.sessionId,
    prompt: [
      {
        type: 'text',
        text: message,
      },
    ],
  })

  expect(response.stopReason).toBe('end_turn')
}

async function openWebSocketStream(
  url: string,
  headers?: Readonly<Record<string, string>>,
): Promise<Stream> {
  const socket = new WebSocket(url, headers ? { headers } : undefined)

  await new Promise<void>((resolve, reject) => {
    const handleOpen = () => {
      socket.off('error', handleError)
      resolve()
    }
    const handleError = (error: Error) => {
      socket.off('open', handleOpen)
      reject(error)
    }
    socket.once('open', handleOpen)
    socket.once('error', handleError)
  })

  return {
    readable: new ReadableStream({
      start(controller) {
        const handleMessage = (data: RawData) => {
          controller.enqueue(JSON.parse(decodeRawData(data)))
        }
        const handleClose = () => {
          cleanup()
          controller.close()
        }
        const handleError = (error: Error) => {
          cleanup()
          controller.error(error)
        }
        const cleanup = () => {
          socket.off('message', handleMessage)
          socket.off('close', handleClose)
          socket.off('error', handleError)
        }

        socket.on('message', handleMessage)
        socket.on('close', handleClose)
        socket.on('error', handleError)
      },
    }),
    writable: new WritableStream({
      write(message) {
        return new Promise<void>((resolve, reject) => {
          socket.send(JSON.stringify(message), (error) => {
            if (error) {
              reject(error)
              return
            }
            resolve()
          })
        })
      },
      close() {
        socket.close()
      },
      abort() {
        socket.close()
      },
    }),
  }
}

function decodeRawData(data: RawData): string {
  if (typeof data === 'string') {
    return data
  }

  if (Buffer.isBuffer(data)) {
    return data.toString('utf8')
  }

  if (Array.isArray(data)) {
    return Buffer.concat(data).toString('utf8')
  }

  if (data instanceof ArrayBuffer) {
    return Buffer.from(data).toString('utf8')
  }

  return Buffer.concat(data).toString('utf8')
}

async function reservePort(): Promise<number> {
  return await new Promise<number>((resolvePort, reject) => {
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

async function waitForHttpOk(url: string, label: string, timeoutMs = 10_000): Promise<void> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url)
      if (response.ok) {
        return
      }
    } catch {
      // Keep polling until the service is listening.
    }
    await sleep(100)
  }
  throw new Error(`timed out waiting for ${label} at ${url}`)
}

async function stopChild(
  child: ChildProcessWithoutNullStreams | undefined,
  signal: NodeJS.Signals = 'SIGTERM',
): Promise<void> {
  if (!child || child.exitCode !== null || child.signalCode !== null) {
    return
  }

  child.kill(signal)
  await Promise.race([
    new Promise<void>((resolve) => {
      child.once('exit', () => resolve())
    }),
    sleep(5_000),
  ])
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}
