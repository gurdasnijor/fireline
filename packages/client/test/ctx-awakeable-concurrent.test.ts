import {
  execFileSync,
  spawn,
  type ChildProcessWithoutNullStreams,
} from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { createServer } from 'node:net'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { DurableStream, stream } from '@durable-streams/client'
import type { RequestId, SessionId } from '@agentclientprotocol/sdk'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import {
  AwakeableAlreadyResolvedError,
  completionKeyStorageKey,
  promptCompletionKey,
  rejectAwakeable,
  resolveAwakeable,
  workflowContext,
} from '../src/workflow/index.js'

type StreamRow = {
  readonly type: string
  readonly key: string
  readonly value?: Record<string, unknown>
}

type PromptKey = ReturnType<typeof promptCompletionKey>

type TerminalRow = StreamRow & {
  readonly value?: Record<string, unknown> & {
    readonly kind?: 'awakeable_resolved' | 'awakeable_rejected'
    readonly value?: unknown
    readonly error?: unknown
  }
}

type SettledOutcome<T> =
  | { readonly status: 'resolved'; readonly value: T }
  | { readonly status: 'rejected'; readonly error: Error }

type RaceResolution = {
  readonly winner: string
}

type DuplicateRaceObservation = {
  readonly key: PromptKey
  readonly streamUrl: string
  readonly rows: readonly TerminalRow[]
  readonly firstValue: RaceResolution
  readonly secondValue: RaceResolution
  readonly waiterValue: RaceResolution
}

const repoRoot = fileURLToPath(new URL('../../../', import.meta.url))
const cargoTargetDir = resolve(repoRoot, process.env.CARGO_TARGET_DIR ?? 'target')
const firelineStreamsBin = resolve(cargoTargetDir, 'debug', 'fireline-streams')

let streamsPort = 0
let streamsProcess: ChildProcessWithoutNullStreams | undefined

describe('workflow ctx.awakeable concurrent semantics', () => {
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
    'concurrent resolveAwakeable callers converge to one durable winner and replay the first duplicate completion',
    async () => {
      const observed = await raceUntilDuplicateResolutions(24)

      expect(observed.rows).toHaveLength(2)
      expect(observed.waiterValue).toEqual(observed.firstValue)
      expect(observed.waiterValue).not.toEqual(observed.secondValue)

      await expect(
        workflowContext({
          stateStreamUrl: observed.streamUrl,
        }).awakeable<RaceResolution>(observed.key).promise,
      ).resolves.toEqual(observed.firstValue)
    },
    30_000,
  )

  it('sequential resolve -> resolve rejects the second completion and keeps the first value', async () => {
    const streamUrl = await createJsonStateStream('ctx-awakeable-double-resolve')
    const key = newPromptKey()
    const winner = { winner: 'resolve-1' }
    const loser = { winner: 'resolve-2' }
    const awakeable = workflowContext({ stateStreamUrl: streamUrl }).awakeable<RaceResolution>(key)

    await waitForAwakeableEvent(streamUrl, key, 'awakeable_waiting')

    await resolveAwakeable({ streamUrl, key, value: winner })
    await expect(awakeable.promise).resolves.toEqual(winner)
    await expect(
      resolveAwakeable({ streamUrl, key, value: loser }),
    ).rejects.toBeInstanceOf(AwakeableAlreadyResolvedError)

    expect(await readTerminalRows(streamUrl, key)).toMatchObject([
      {
        key: `${completionKeyStorageKey(key)}:resolved`,
        value: {
          kind: 'awakeable_resolved',
          value: winner,
        },
      },
    ])
  })

  it('sequential resolve -> reject returns AlreadyResolved and keeps the resolved waiter outcome', async () => {
    const streamUrl = await createJsonStateStream('ctx-awakeable-resolve-reject')
    const key = newPromptKey()
    const winner = { winner: 'resolve-first' }
    const loser = { reason: 'reject-second' }
    const awakeable = workflowContext({ stateStreamUrl: streamUrl }).awakeable<RaceResolution>(key)

    await waitForAwakeableEvent(streamUrl, key, 'awakeable_waiting')

    await resolveAwakeable({ streamUrl, key, value: winner })
    await expect(awakeable.promise).resolves.toEqual(winner)
    await expect(
      rejectAwakeable({ streamUrl, key, error: loser }),
    ).rejects.toBeInstanceOf(AwakeableAlreadyResolvedError)

    expect(await readTerminalRows(streamUrl, key)).toMatchObject([
      {
        key: `${completionKeyStorageKey(key)}:resolved`,
        value: {
          kind: 'awakeable_resolved',
          value: winner,
        },
      },
    ])
  })

  it('sequential reject -> resolve returns AlreadyResolved and keeps the rejected waiter outcome', async () => {
    const streamUrl = await createJsonStateStream('ctx-awakeable-reject-resolve')
    const key = newPromptKey()
    const winner = { reason: 'reject-first' }
    const loser = { winner: 'resolve-second' }
    const awakeable = workflowContext({ stateStreamUrl: streamUrl }).awakeable<RaceResolution>(key)

    await waitForAwakeableEvent(streamUrl, key, 'awakeable_waiting')

    await rejectAwakeable({ streamUrl, key, error: winner })
    await expect(awakeable.promise).rejects.toThrow(JSON.stringify(winner))
    await expect(
      resolveAwakeable({ streamUrl, key, value: loser }),
    ).rejects.toBeInstanceOf(AwakeableAlreadyResolvedError)

    expect(await readTerminalRows(streamUrl, key)).toMatchObject([
      {
        key: `${completionKeyStorageKey(key)}:rejected`,
        value: {
          kind: 'awakeable_rejected',
          error: winner,
        },
      },
    ])
  })

  it('concurrent resolve -> resolve preserves first-wins semantics for the waiter and replay', async () => {
    const streamUrl = await createJsonStateStream('ctx-awakeable-concurrent-resolve')
    const key = newPromptKey()
    const awakeable = workflowContext({ stateStreamUrl: streamUrl }).awakeable<RaceResolution>(key)

    await waitForAwakeableEvent(streamUrl, key, 'awakeable_waiting')

    const results = await settleConcurrent([
      () => resolveAwakeable({ streamUrl, key, value: { winner: 'resolve-a' } }),
      () => resolveAwakeable({ streamUrl, key, value: { winner: 'resolve-b' } }),
    ])

    expectConcurrentCompletionResults(results)

    await waitForTerminalRows(streamUrl, key, 1)
    await sleep(100)
    const rows = await readTerminalRows(streamUrl, key)
    expect(rows.length === 1 || rows.length === 2).toBe(true)
    expect(rows[0]?.value?.kind).toBe('awakeable_resolved')

    const waiterValue = await awakeable.promise
    expect(waiterValue).toEqual(rows[0]?.value?.value)
    if (rows.length === 2) {
      expect(waiterValue).not.toEqual(rows[1]?.value?.value)
    }

    await expect(
      workflowContext({ stateStreamUrl: streamUrl }).awakeable<RaceResolution>(key).promise,
    ).resolves.toEqual(rows[0]?.value?.value)
  })

  it('concurrent resolve + reject preserve first-terminal semantics for the waiter and replay', async () => {
    const streamUrl = await createJsonStateStream('ctx-awakeable-concurrent-resolve-reject')
    const key = newPromptKey()
    const awakeable = workflowContext({ stateStreamUrl: streamUrl }).awakeable<RaceResolution>(key)

    await waitForAwakeableEvent(streamUrl, key, 'awakeable_waiting')

    const results = await settleConcurrent([
      () => resolveAwakeable({ streamUrl, key, value: { winner: 'resolve' } }),
      () => rejectAwakeable({ streamUrl, key, error: { winner: 'reject' } }),
    ])

    expectConcurrentCompletionResults(results)

    await waitForTerminalRows(streamUrl, key, 1)
    await sleep(100)
    const rows = await readTerminalRows(streamUrl, key)
    expect(rows.length === 1 || rows.length === 2).toBe(true)
    expect(rows[0]?.value?.kind).toMatch(/awakeable_(resolved|rejected)/)

    const settled = await settleAwakeable(awakeable.promise)
    expectSettledMatchesTerminal(settled, rows[0]!)

    const replayed = await settleAwakeable(
      workflowContext({ stateStreamUrl: streamUrl }).awakeable<RaceResolution>(key).promise,
    )
    expectSettledMatchesTerminal(replayed, rows[0]!)
  })
})

async function createJsonStateStream(label: string): Promise<string> {
  const streamUrl = `http://127.0.0.1:${streamsPort}/v1/stream/${label}-${randomUUID()}`
  const handle = new DurableStream({
    url: streamUrl,
    contentType: 'application/json',
  })
  await handle.create({ contentType: 'application/json' })
  return streamUrl
}

function newPromptKey(): PromptKey {
  return promptCompletionKey({
    sessionId: `session-${randomUUID()}` as SessionId,
    requestId: `request-${randomUUID()}` as RequestId,
  })
}

async function raceUntilDuplicateResolutions(
  maxAttempts: number,
): Promise<DuplicateRaceObservation> {
  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    const streamUrl = await createJsonStateStream(`ctx-awakeable-duplicate-race-${attempt}`)
    const key = newPromptKey()
    const waiter = workflowContext({ stateStreamUrl: streamUrl }).awakeable<RaceResolution>(key)

    await waitForAwakeableEvent(streamUrl, key, 'awakeable_waiting')

    const results = await settleConcurrent([
      () =>
        resolveAwakeable({
          streamUrl,
          key,
          value: { winner: `resolver-a-${attempt}` },
        }),
      () =>
        resolveAwakeable({
          streamUrl,
          key,
          value: { winner: `resolver-b-${attempt}` },
        }),
    ])

    const waiterValue = await waiter.promise
    await sleep(100)

    const rows = await readTerminalRows(streamUrl, key)
    if (rows.length === 2) {
      expectDuplicateRaceResults(results)

      return {
        key,
        streamUrl,
        rows,
        firstValue: rows[0]!.value?.value as RaceResolution,
        secondValue: rows[1]!.value?.value as RaceResolution,
        waiterValue,
      }
    }
  }

  throw new Error(
    `did not observe duplicate awakeable_resolved rows after ${maxAttempts} concurrent attempts`,
  )
}

async function settleConcurrent(
  actions: readonly [() => Promise<void>, () => Promise<void>],
): Promise<readonly [PromiseSettledResult<void>, PromiseSettledResult<void>]> {
  const syncStart = barrier(actions.length)
  const results = await Promise.allSettled(
    actions.map(async (action) => {
      await syncStart()
      await action()
    }),
  )
  return results as [PromiseSettledResult<void>, PromiseSettledResult<void>]
}

function barrier(parties: number): () => Promise<void> {
  let waiting = 0
  let releaseBarrier!: () => void
  const release = new Promise<void>((resolveBarrier) => {
    releaseBarrier = resolveBarrier
  })

  return async () => {
    waiting += 1
    if (waiting === parties) {
      releaseBarrier()
    }
    await release
  }
}

function expectConcurrentCompletionResults(
  results: readonly [PromiseSettledResult<void>, PromiseSettledResult<void>],
): void {
  const fulfilled = results.filter((result) => result.status === 'fulfilled')
  const rejected = results.filter((result) => result.status === 'rejected')

  expect(fulfilled.length === 1 || fulfilled.length === 2).toBe(true)
  expect(rejected.length === 0 || rejected.length === 1).toBe(true)
  if (rejected.length === 1) {
    expect(rejected[0]?.reason).toBeInstanceOf(AwakeableAlreadyResolvedError)
  }
}

function expectDuplicateRaceResults(
  results: readonly [PromiseSettledResult<void>, PromiseSettledResult<void>],
): void {
  const rejected = results.filter((result) => result.status === 'rejected')
  const fulfilled = results.filter((result) => result.status === 'fulfilled')

  expect(fulfilled.length === 2 || (fulfilled.length === 1 && rejected.length === 1)).toBe(true)
  if (rejected.length === 1) {
    expect(rejected[0]?.reason).toBeInstanceOf(AwakeableAlreadyResolvedError)
  }
}

async function settleAwakeable<T>(promise: Promise<T>): Promise<SettledOutcome<T>> {
  try {
    return { status: 'resolved', value: await promise }
  } catch (error) {
    return {
      status: 'rejected',
      error: error instanceof Error ? error : new Error(String(error)),
    }
  }
}

function expectSettledMatchesTerminal<T>(
  settled: SettledOutcome<T>,
  terminal: TerminalRow,
): void {
  if (terminal.value?.kind === 'awakeable_resolved') {
    expect(settled).toEqual({
      status: 'resolved',
      value: terminal.value.value as T,
    })
    return
  }

  expect(terminal.value?.kind).toBe('awakeable_rejected')
  expect(settled.status).toBe('rejected')
  if (settled.status === 'rejected') {
    expect(settled.error.message).toContain(
      JSON.stringify(terminal.value?.error ?? null),
    )
  }
}

async function waitForAwakeableEvent(
  stateStreamUrl: string,
  key: PromptKey,
  kind: 'awakeable_waiting' | 'awakeable_resolved' | 'awakeable_rejected',
  timeoutMs = 10_000,
): Promise<StreamRow> {
  const suffix =
    kind === 'awakeable_waiting'
      ? 'waiting'
      : kind === 'awakeable_resolved'
        ? 'resolved'
        : 'rejected'

  return await waitForDefined(async () => {
    const rows = await readRows(stateStreamUrl)
    return rows.find(
      (row) =>
        row.type === 'awakeable' &&
        row.key === `${completionKeyStorageKey(key)}:${suffix}` &&
        row.value?.kind === kind,
    )
  }, timeoutMs, `awakeable ${kind}`)
}

async function waitForTerminalRows(
  stateStreamUrl: string,
  key: PromptKey,
  count: number,
  timeoutMs = 10_000,
): Promise<readonly TerminalRow[]> {
  return await waitForDefined(async () => {
    const rows = await readTerminalRows(stateStreamUrl, key)
    return rows.length >= count ? rows : undefined
  }, timeoutMs, 'awakeable terminal rows')
}

async function readTerminalRows(
  streamUrl: string,
  key: PromptKey,
): Promise<readonly TerminalRow[]> {
  const storageKey = completionKeyStorageKey(key)
  const rows = await readRows(streamUrl)
  return rows.filter(
    (row): row is TerminalRow =>
      row.type === 'awakeable' &&
      (row.value?.kind === 'awakeable_resolved' ||
        row.value?.kind === 'awakeable_rejected') &&
      (row.key === `${storageKey}:resolved` || row.key === `${storageKey}:rejected`),
  )
}

async function readRows(streamUrl: string): Promise<readonly StreamRow[]> {
  const response = await stream<StreamRow>({
    url: streamUrl,
    json: true,
    live: false,
    offset: '-1',
  })
  return await response.json()
}

async function waitForDefined<T>(
  getValue: () => Promise<T | undefined>,
  timeoutMs: number,
  label: string,
): Promise<T> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const value = await getValue()
    if (value !== undefined) {
      return value
    }
    await sleep(50)
  }
  throw new Error(`timed out waiting for ${label}`)
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolveSleep) => {
    setTimeout(resolveSleep, ms)
  })
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
    new Promise<void>((resolveExit) => {
      child.once('exit', () => resolveExit())
    }),
    sleep(5_000),
  ])
}
