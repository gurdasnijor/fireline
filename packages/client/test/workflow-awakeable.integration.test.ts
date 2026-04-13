import {
  execFileSync,
  spawn,
  type ChildProcessWithoutNullStreams,
} from 'node:child_process'
import { createServer } from 'node:net'
import { randomUUID } from 'node:crypto'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { DurableStream } from '@durable-streams/client'
import type { RequestId, SessionId } from '@agentclientprotocol/sdk'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import {
  AwakeableAlreadyResolvedError,
  promptCompletionKey,
  resolveAwakeable,
  workflowContext,
} from '../src/index.js'

type StreamRow = {
  readonly type: string
  readonly key: string
  readonly value?: Record<string, unknown>
}

const repoRoot = fileURLToPath(new URL('../../../', import.meta.url))
const cargoTargetDir = resolve(repoRoot, process.env.CARGO_TARGET_DIR ?? 'target')
const firelineStreamsBin = join(cargoTargetDir, 'debug', 'fireline-streams')

let streamsPort = 0
let streamsProcess: ChildProcessWithoutNullStreams | undefined

describe('workflow awakeable replay integration', () => {
  beforeAll(async () => {
    execFileSync('cargo', ['build', '--quiet', '--bin', 'fireline-streams'], {
      cwd: repoRoot,
      stdio: 'inherit',
      env: process.env,
    })

    streamsPort = await reservePort()
    streamsProcess = spawn(firelineStreamsBin, ['--port', String(streamsPort)], {
      cwd: repoRoot,
      stdio: 'inherit',
    })
    await waitForHttpOk(`http://127.0.0.1:${streamsPort}/healthz`, 'fireline-streams')
  }, 30_000)

  afterAll(async () => {
    await stopChild(streamsProcess)
  })

  it(
    'replays an already-resolved awakeable without appending a second waiting row',
    async () => {
      const streamUrl = stateStreamUrl(`workflow-awakeable-resolved-${randomUUID()}`)
      await ensureJsonStreamExists(streamUrl)
      const key = promptCompletionKey({
        sessionId: 'ts-session-resolved' as SessionId,
        requestId: 'ts-request-resolved' as RequestId,
      })

      await resolveAwakeable({
        streamUrl,
        key,
        value: { approved: true },
        traceContext: {
          traceparent: '00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01',
        },
      })

      const approval = workflowContext({ stateStreamUrl: streamUrl }).awakeable<{
        approved: boolean
      }>(key)
      await expect(approval.promise).resolves.toEqual({ approved: true })

      const rows = await readRows(streamUrl)
      expect(countKind(rows, key, 'awakeable_waiting')).toBe(0)
      expect(countKind(rows, key, 'awakeable_resolved')).toBe(1)
    },
    20_000,
  )

  it(
    'reuses an existing waiting row on replay and resolves through the live subscriber path',
    async () => {
      const streamUrl = stateStreamUrl(`workflow-awakeable-pending-${randomUUID()}`)
      await ensureJsonStreamExists(streamUrl)
      const key = promptCompletionKey({
        sessionId: 'ts-session-pending' as SessionId,
        requestId: 'ts-request-pending' as RequestId,
      })

      await appendWaiting(streamUrl, key, {
        traceparent: '00-cccccccccccccccccccccccccccccccc-dddddddddddddddd-01',
      })

      const approval = workflowContext({ stateStreamUrl: streamUrl }).awakeable<boolean>(key)
      const completion = resolveAwakeable({
        streamUrl,
        key,
        value: true,
        traceContext: {
          traceparent: '00-cccccccccccccccccccccccccccccccc-dddddddddddddddd-01',
        },
      })

      await expect(approval.promise).resolves.toBe(true)
      await expect(completion).resolves.toBeUndefined()

      const rows = await readRows(streamUrl)
      expect(countKind(rows, key, 'awakeable_waiting')).toBe(1)
      expect(countKind(rows, key, 'awakeable_resolved')).toBe(1)
    },
    20_000,
  )

  it(
    'preserves traceparent and converges to one winner across replay/live resolve races',
    async () => {
      const streamUrl = stateStreamUrl(`workflow-awakeable-race-${randomUUID()}`)
      await ensureJsonStreamExists(streamUrl)
      const key = promptCompletionKey({
        sessionId: 'ts-session-race' as SessionId,
        requestId: 'ts-request-race' as RequestId,
      })
      const traceparent = '00-eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-ffffffffffffffff-01'

      await appendWaiting(streamUrl, key, { traceparent })

      const replayed = workflowContext({ stateStreamUrl: streamUrl }).awakeable<{
        winner: number
      }>(key)

      const first = resolveAwakeable({
        streamUrl,
        key,
        value: { winner: 1 },
        traceContext: { traceparent },
      })
      const second = resolveAwakeable({
        streamUrl,
        key,
        value: { winner: 2 },
        traceContext: { traceparent },
      })

      const [firstResult, secondResult] = await Promise.allSettled([first, second])
      expect([firstResult.status, secondResult.status].sort()).toEqual([
        'fulfilled',
        'rejected',
      ])
      const rejected = [firstResult, secondResult].find(
        (result): result is PromiseRejectedResult => result.status === 'rejected',
      )
      expect(rejected?.reason).toBeInstanceOf(AwakeableAlreadyResolvedError)

      await expect(replayed.promise).resolves.toMatchObject({ winner: expect.any(Number) })

      const rows = await readRows(streamUrl)
      expect(countKind(rows, key, 'awakeable_waiting')).toBe(1)
      expect(countKind(rows, key, 'awakeable_resolved')).toBe(1)
      expect(traceparentForKind(rows, key, 'awakeable_waiting')).toBe(traceparent)
      expect(traceparentForKind(rows, key, 'awakeable_resolved')).toBe(traceparent)
    },
    20_000,
  )
})

function stateStreamUrl(name: string): string {
  return `http://127.0.0.1:${streamsPort}/v1/stream/${name}`
}

async function ensureJsonStreamExists(streamUrl: string): Promise<void> {
  const stream = new DurableStream({
    url: streamUrl,
    contentType: 'application/json',
  })
  await stream.create({
    contentType: 'application/json',
  })
}

async function appendWaiting(
  streamUrl: string,
  key: ReturnType<typeof promptCompletionKey>,
  traceContext?: {
    readonly traceparent?: string
    readonly tracestate?: string
    readonly baggage?: string
  },
): Promise<void> {
  const value: Record<string, unknown> = {
    kind: 'awakeable_waiting',
    sessionId: key.sessionId,
    requestId: key.requestId,
    createdAtMs: Date.now(),
  }
  if (traceContext && Object.values(traceContext).some(Boolean)) {
    value._meta = Object.fromEntries(
      Object.entries(traceContext).filter(([, current]) => current !== undefined && current !== ''),
    )
  }

  const stream = new DurableStream({
    url: streamUrl,
    contentType: 'application/json',
  })
  await stream.append(
    JSON.stringify({
      type: 'awakeable',
      key: `prompt:${key.sessionId}:${String(key.requestId)}:waiting`,
      headers: { operation: 'insert' },
      value,
    }),
    {
      contentType: 'application/json',
    },
  )
}

async function readRows(streamUrl: string): Promise<StreamRow[]> {
  const url = new URL(streamUrl)
  url.searchParams.set('offset', '-1')
  const response = await fetch(url)
  if (!response.ok) {
    throw new Error(`failed to read stream ${streamUrl}: ${response.status} ${response.statusText}`)
  }
  return (await response.json()) as StreamRow[]
}

function countKind(
  rows: readonly StreamRow[],
  key: ReturnType<typeof promptCompletionKey>,
  kind: string,
): number {
  const base = `prompt:${key.sessionId}:${String(key.requestId)}`
  return rows.filter(
    (row) =>
      row.type === 'awakeable' &&
      row.value?.kind === kind &&
      row.key === `${base}:${kind === 'awakeable_waiting' ? 'waiting' : 'resolved'}`,
  ).length
}

function traceparentForKind(
  rows: readonly StreamRow[],
  key: ReturnType<typeof promptCompletionKey>,
  kind: string,
): string | undefined {
  const base = `prompt:${key.sessionId}:${String(key.requestId)}`
  return rows.find(
    (row) =>
      row.type === 'awakeable' &&
      row.value?.kind === kind &&
      row.key === `${base}:${kind === 'awakeable_waiting' ? 'waiting' : 'resolved'}`,
  )?.value?._meta?.traceparent as string | undefined
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
      // keep polling
    }
    await sleep(100)
  }
  throw new Error(`timed out waiting for ${label} at ${url}`)
}

async function stopChild(
  child: ChildProcessWithoutNullStreams | undefined,
): Promise<void> {
  if (!child || child.exitCode !== null || child.signalCode !== null) {
    return
  }
  child.kill('SIGTERM')
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
