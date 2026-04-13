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

import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { ChunkRow, FirelineDB } from '@fireline/state'

import {
  FirelineAgent,
  agent,
  compose,
  db,
  middleware,
  sandbox,
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
let handle: FirelineAgent<string> | undefined
let stateDb: FirelineDB | undefined

describe('client.acp.connect', () => {
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
    tempRoot = await mkdtemp(join(tmpdir(), 'fireline-client-acp-'))
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

  afterAll(async () => {
    stateDb?.close()
    stateDb = undefined
    await handle?.stop().catch(() => {})
    handle = undefined
    await stopChild(controlPlaneProcess)
    await stopChild(streamsProcess)
    await rm(tempRoot, { recursive: true, force: true })
  })

  it(
    'connects to a hosted runtime, prompts over ACP, and observes durable state',
    async () => {
      handle = await compose(
        sandbox({ provider: 'local' }),
        middleware([]),
        agent([firelineTestyBin]),
      ).start({
        serverUrl: `http://127.0.0.1:${controlPlanePort}`,
        name: `ts-acp-${randomUUID()}`,
        stateStream: `ts-acp-state-${randomUUID()}`,
      })

      stateDb = await db({ stateStreamUrl: handle.state.url })
      const acp = await handle.connect('acp.test.ts')

      try {
        const session = await acp.newSession({
          cwd: repoRoot,
          mcpServers: [],
        })

        const response = await acp.prompt({
          sessionId: session.sessionId,
          prompt: [
            {
              type: 'text',
              text: 'hello from TypeScript',
            },
          ],
        })

        expect(response.stopReason).toBe('end_turn')

        const promptRequest = await waitForDefined(
          () =>
            stateDb?.collections.promptRequests.toArray.find(
              (turn) => turn.sessionId === session.sessionId && turn.state === 'completed',
            ),
          5_000,
        )

        expect(promptRequest.text).toBe('hello from TypeScript')

        const chunk = await waitForDefined(
          () =>
            stateDb?.collections.chunks.toArray.find(
              (entry) =>
                entry.sessionId === promptRequest.sessionId &&
                entry.requestId === promptRequest.requestId &&
                isTextAgentMessageChunk(entry) &&
                entry.update.content.text.includes('Hello'),
            ),
          5_000,
        )

        const canonicalChunk = {
          sessionId: chunk.sessionId,
          requestId: chunk.requestId,
          toolCallId: chunk.toolCallId,
          update: chunk.update,
          createdAt: chunk.createdAt,
        } satisfies ChunkRow

        expect(canonicalChunk.sessionId).toBe(session.sessionId)
        expect(canonicalChunk.requestId).toBe(promptRequest.requestId)
        expect(canonicalChunk.update.sessionUpdate).toBe('agent_message_chunk')
        expect(canonicalChunk.update.content.type).toBe('text')
        expect(canonicalChunk.update.content.text).toContain('Hello')
        expect(stripCollectionMetadata(chunk)).toEqual(canonicalChunk)
      } finally {
        await acp.close()
        stateDb?.close()
        stateDb = undefined
        await handle?.stop()
        handle = undefined
      }
    },
    20_000,
  )
})

function isTextAgentMessageChunk(
  row: ChunkRow,
): row is ChunkRow & {
  readonly update: Extract<ChunkRow['update'], { readonly sessionUpdate: 'agent_message_chunk' }>
} {
  return row.update.sessionUpdate === 'agent_message_chunk' && row.update.content.type === 'text'
}

async function waitForDefined<T>(getValue: () => T | undefined, timeoutMs: number): Promise<T> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const value = getValue()
    if (value !== undefined) {
      return value
    }
    await sleep(50)
  }
  throw new Error('timed out waiting for expected value')
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}

function stripCollectionMetadata<T extends object>(row: T): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(row).filter(([key]) => !key.startsWith('$')),
  )
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
