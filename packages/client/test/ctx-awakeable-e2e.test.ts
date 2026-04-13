import {
  execFileSync,
  spawn,
  type ChildProcessWithoutNullStreams,
} from 'node:child_process'
import { createServer } from 'node:net'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { randomUUID } from 'node:crypto'

import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { RequestId, SessionId } from '@agentclientprotocol/sdk'
import { stream } from '@durable-streams/client'

import {
  type FirelineAgent,
  agent,
  completionKeyStorageKey,
  compose,
  promptCompletionKey,
  rejectAwakeable,
  resolveAwakeable,
  sandbox,
  workflowContext,
} from '../src/index.js'
import { middleware } from '../src/sandbox.js'

type StreamRow = {
  readonly type: string
  readonly key: string
  readonly value?: Record<string, unknown>
}

type PromptRequestRow = {
  readonly sessionId: string
  readonly requestId: string
  readonly text: string
  readonly _meta?: {
    readonly traceparent?: string
    readonly tracestate?: string
    readonly baggage?: string
  }
}

const repoRoot = fileURLToPath(new URL('../../../', import.meta.url))
const cargoTargetDir = resolve(repoRoot, process.env.CARGO_TARGET_DIR ?? 'target')
const firelineBin = resolve(cargoTargetDir, 'debug', 'fireline')
const firelineStreamsBin = resolve(cargoTargetDir, 'debug', 'fireline-streams')
const firelineTestyBin = resolve(cargoTargetDir, 'debug', 'fireline-testy')

let streamsPort = 0
let streamsProcess: ChildProcessWithoutNullStreams | undefined

describe('workflow ctx.awakeable e2e', () => {
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
    'resolves a prompt-scoped ctx.awakeable through the TS workflow surface and preserves trace context',
    async () => {
      const controlPlane = await startControlPlane()
      let handle: FirelineAgent<string> | undefined
      let acp: Awaited<ReturnType<FirelineAgent['connect']>> | undefined

      try {
        handle = await startRuntime(controlPlane.serverUrl, `ctx-awakeable-resolve-${randomUUID()}`)
        acp = await handle.connect('ctx-awakeable-e2e.resolve')

        const session = await acp.newSession({
          cwd: repoRoot,
          mcpServers: [],
        })
        const promptTraceparent =
          '00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01'
        const promptText = `ctx.awakeable resolve ${randomUUID()}`
        await acp.prompt({
          sessionId: session.sessionId,
          prompt: [{ type: 'text', text: promptText }],
          _meta: { traceparent: promptTraceparent },
        } as any)

        const promptRequest = await waitForPromptRequest(
          handle.state.url,
          session.sessionId,
          promptText,
        )
        const traceparent =
          promptRequest._meta?.traceparent ?? promptTraceparent
        const key = promptCompletionKey({
          sessionId: session.sessionId as SessionId,
          requestId: promptRequest.requestId as RequestId,
        })

        const ctx = workflowContext({ stateStreamUrl: handle.state.url })
        const awakeable = ctx.awakeable<{ approved: boolean; reviewer: string }>(key)
        await waitForAwakeableEvent(handle.state.url, key, 'awakeable_waiting')

        await resolveAwakeable({
          streamUrl: handle.state.url,
          key,
          value: { approved: true, reviewer: 'ops-oncall' },
          traceContext: {
            traceparent,
            tracestate: 'vendor=value',
            baggage: 'scope=ctx-awakeable-e2e',
          },
        })

        await expect(awakeable.promise).resolves.toEqual({
          approved: true,
          reviewer: 'ops-oncall',
        })

        const completion = await waitForAwakeableEvent(
          handle.state.url,
          key,
          'awakeable_resolved',
        )
        expect(completion.value?.requestId).toBe(promptRequest.requestId)
        expect(completion.value?._meta).toMatchObject({
          traceparent,
          tracestate: 'vendor=value',
          baggage: 'scope=ctx-awakeable-e2e',
        })
      } finally {
        await acp?.close().catch(() => {})
        await handle?.stop().catch(() => {})
        await stopChild(controlPlane.process)
      }
    },
    30_000,
  )

  it(
    'keeps a pending ctx.awakeable alive across host restart and resolves deterministically after the restart',
    async () => {
      let controlPlane = await startControlPlane()
      let handle: FirelineAgent<string> | undefined
      let acp: Awaited<ReturnType<FirelineAgent['connect']>> | undefined

      try {
        handle = await startRuntime(controlPlane.serverUrl, `ctx-awakeable-restart-${randomUUID()}`)
        acp = await handle.connect('ctx-awakeable-e2e.restart')

        const session = await acp.newSession({
          cwd: repoRoot,
          mcpServers: [],
        })
        const promptTraceparent =
          '00-cccccccccccccccccccccccccccccccc-dddddddddddddddd-01'
        const promptText = `ctx.awakeable restart ${randomUUID()}`
        await acp.prompt({
          sessionId: session.sessionId,
          prompt: [{ type: 'text', text: promptText }],
          _meta: { traceparent: promptTraceparent },
        } as any)

        const promptRequest = await waitForPromptRequest(
          handle.state.url,
          session.sessionId,
          promptText,
        )
        const traceparent =
          promptRequest._meta?.traceparent ?? promptTraceparent
        const key = promptCompletionKey({
          sessionId: session.sessionId as SessionId,
          requestId: promptRequest.requestId as RequestId,
        })

        const awakeable = workflowContext({
          stateStreamUrl: handle.state.url,
        }).awakeable<{ approved: boolean; reviewer: string }>(key)
        await waitForAwakeableEvent(handle.state.url, key, 'awakeable_waiting')

        await acp.close().catch(() => {})
        acp = undefined
        await stopChild(controlPlane.process, 'SIGTERM')
        controlPlane = await startControlPlane()

        await resolveAwakeable({
          streamUrl: handle.state.url,
          key,
          value: { approved: true, reviewer: 'restart-replayer' },
          traceContext: {
            traceparent,
            baggage: 'phase=restart',
          },
        })

        await expect(awakeable.promise).resolves.toEqual({
          approved: true,
          reviewer: 'restart-replayer',
        })

        const completions = await waitForTerminalCount(handle.state.url, key, 1)
        expect(completions).toBe(1)
      } finally {
        await handle?.stop().catch(() => {})
        await stopChild(controlPlane.process)
      }
    },
    30_000,
  )

  it(
    'rejects a prompt-scoped ctx.awakeable through the TS workflow surface and preserves rejection trace context',
    async () => {
      const controlPlane = await startControlPlane()
      let handle: FirelineAgent<string> | undefined
      let acp: Awaited<ReturnType<FirelineAgent['connect']>> | undefined

      try {
        handle = await startRuntime(controlPlane.serverUrl, `ctx-awakeable-reject-${randomUUID()}`)
        acp = await handle.connect('ctx-awakeable-e2e.reject')

        const session = await acp.newSession({
          cwd: repoRoot,
          mcpServers: [],
        })
        const promptTraceparent =
          '00-eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-ffffffffffffffff-01'
        const promptText = `ctx.awakeable reject ${randomUUID()}`
        await acp.prompt({
          sessionId: session.sessionId,
          prompt: [{ type: 'text', text: promptText }],
          _meta: { traceparent: promptTraceparent },
        } as any)

        const promptRequest = await waitForPromptRequest(
          handle.state.url,
          session.sessionId,
          promptText,
        )
        const traceparent =
          promptRequest._meta?.traceparent ?? promptTraceparent
        const key = promptCompletionKey({
          sessionId: session.sessionId as SessionId,
          requestId: promptRequest.requestId as RequestId,
        })

        const awakeable = workflowContext({
          stateStreamUrl: handle.state.url,
        }).awakeable<{ approved: boolean }>(key)
        await waitForAwakeableEvent(handle.state.url, key, 'awakeable_waiting')

        await rejectAwakeable({
          streamUrl: handle.state.url,
          key,
          error: { reason: 'policy denied' },
          traceContext: {
            traceparent,
            baggage: 'phase=reject',
          },
        })

        await expect(awakeable.promise).rejects.toThrow('policy denied')

        const completion = await waitForAwakeableEvent(
          handle.state.url,
          key,
          'awakeable_rejected',
        )
        expect(completion.value?.error).toEqual({ reason: 'policy denied' })
        expect(completion.value?._meta).toMatchObject({
          traceparent,
          baggage: 'phase=reject',
        })
      } finally {
        await acp?.close().catch(() => {})
        await handle?.stop().catch(() => {})
        await stopChild(controlPlane.process)
      }
    },
    30_000,
  )
})

async function startRuntime(
  serverUrl: string,
  stateStream: string,
): Promise<FirelineAgent<string>> {
  return await compose(
    sandbox({ provider: 'local' }),
    middleware([]),
    agent([firelineTestyBin]),
  ).start({
    serverUrl,
    name: `ctx-awakeable-${randomUUID()}`,
    stateStream,
  })
}

async function startControlPlane(): Promise<{
  readonly process: ChildProcessWithoutNullStreams
  readonly serverUrl: string
}> {
  const port = await reservePort()
  const process = spawn(
    firelineBin,
    [
      '--control-plane',
      '--host',
      '127.0.0.1',
      '--port',
      String(port),
      '--durable-streams-url',
      `http://127.0.0.1:${streamsPort}/v1/stream`,
    ],
    {
      cwd: repoRoot,
      stdio: 'inherit',
    },
  )
  const serverUrl = `http://127.0.0.1:${port}`
  await waitForHttpOk(`${serverUrl}/healthz`, 'fireline')
  return { process, serverUrl }
}

async function waitForPromptRequest(
  stateStreamUrl: string,
  sessionId: string,
  text: string,
  timeoutMs = 10_000,
): Promise<PromptRequestRow> {
  return await waitForDefined(async () => {
    const rows = await readRows(stateStreamUrl)
    return rows
      .filter((row) => row.type === 'prompt_request')
      .map((row) => row.value as PromptRequestRow | undefined)
      .find(
        (row) =>
          row?.sessionId === sessionId &&
          row.text === text,
      )
  }, timeoutMs, 'prompt_request row')
}

async function waitForAwakeableEvent(
  stateStreamUrl: string,
  key: ReturnType<typeof promptCompletionKey>,
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

async function waitForTerminalCount(
  stateStreamUrl: string,
  key: ReturnType<typeof promptCompletionKey>,
  count: number,
  timeoutMs = 10_000,
): Promise<number> {
  return await waitForDefined(async () => {
    const rows = await readRows(stateStreamUrl)
    const terminalCount = rows.filter(
      (row) =>
        row.type === 'awakeable' &&
        (row.value?.kind === 'awakeable_resolved' ||
          row.value?.kind === 'awakeable_rejected') &&
        (row.key === `${completionKeyStorageKey(key)}:resolved` ||
          row.key === `${completionKeyStorageKey(key)}:rejected`),
    ).length
    return terminalCount >= count ? terminalCount : undefined
  }, timeoutMs, 'awakeable terminal count')
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
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
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
    new Promise<void>((resolve) => {
      child.once('exit', () => resolve())
    }),
    sleep(5_000),
  ])
}
