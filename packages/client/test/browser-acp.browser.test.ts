import { afterEach, describe, expect, it } from 'vitest'
import type { SessionNotification } from '@agentclientprotocol/sdk'

import { createBrowserFirelineClient } from '../src/browser.js'

describe('browser client ACP + StreamDB', () => {
  afterEach(async () => {
    await deleteRuntime(`${window.location.origin}/api`).catch(() => undefined)
  })

  it(
    'creates a runtime, prompts over ACP from the browser, reconnects via loadSession, and observes durable state',
    async () => {
      const harnessApiBase = `${window.location.origin}/api`
      const client = createBrowserFirelineClient()
      const promptText1 = `hello from browser client ${crypto.randomUUID()}`
      const promptText2 = `hello again from browser client ${crypto.randomUUID()}`

      await deleteRuntime(harnessApiBase).catch(() => undefined)

      const created = await fetchJson<{ runtime: HarnessRuntime }>(`${harnessApiBase}/runtime`, {
        method: 'POST',
        body: JSON.stringify({ agentId: 'fireline-testy-load' }),
      })

      const runtime = await waitForRuntimeReady(harnessApiBase, created.runtime.runtimeKey)
      const db = client.state.open({ stateStreamUrl: runtime.state.url })
      await db.preload()

      const firstConnection = await connectWithRetry(client, runtime.acp.url)
      const firstUpdates = firstConnection.updates()[Symbol.asyncIterator]()

      try {
        const init = await firstConnection.initialize({
          clientCapabilities: { fs: { readTextFile: false } },
          clientInfo: {
            name: '@fireline/client/browser-test',
            version: '0.0.1',
            title: 'Fireline Browser Client Test',
          },
        })

        expect(init.protocolVersion).toBe(1)
        expect(runtime.status).toBe('ready')
        expect(runtime.runtimeId).toMatch(/^fireline:browser-harness:/)

        const session = await firstConnection.connection.newSession({
          cwd: '/',
          mcpServers: [],
        })

        const firstPromptPromise = firstConnection.connection.prompt({
          sessionId: session.sessionId,
          prompt: [{ type: 'text', text: promptText1 }],
        })

        const firstUpdate = await waitForMatchingUpdate(firstUpdates, (notification) => {
          return (
            notification.sessionId === session.sessionId &&
            notification.update.sessionUpdate === 'agent_message_chunk'
          )
        })

        const firstResponse = await firstPromptPromise

        expect(firstResponse.stopReason).toBe('end_turn')
        expect(firstUpdate.sessionId).toBe(session.sessionId)
        if (firstUpdate.update.sessionUpdate !== 'agent_message_chunk') {
          throw new Error(`unexpected update kind ${firstUpdate.update.sessionUpdate}`)
        }
        expect(firstUpdate.update.content.type).toBe('text')
        expect(firstUpdate.update.content.text).toContain('Hello')

        const firstPromptTurn = await waitFor(
          () =>
            db.collections.promptTurns.toArray.find(
              (turn) =>
                turn.sessionId === session.sessionId &&
                turn.state === 'completed' &&
                turn.text === promptText1,
            ),
          5_000,
        )

        expect(firstPromptTurn).toBeDefined()

        await firstConnection.close()

        const secondConnection = await connectWithRetry(client, runtime.acp.url)
        const secondUpdates = secondConnection.updates()[Symbol.asyncIterator]()

        try {
          await secondConnection.initialize({
            clientCapabilities: { fs: { readTextFile: false } },
            clientInfo: {
              name: '@fireline/client/browser-test',
              version: '0.0.1',
              title: 'Fireline Browser Client Test',
            },
          })

          const load = await secondConnection.connection.loadSession({
            sessionId: session.sessionId,
            cwd: '/',
            mcpServers: [],
          })

          expect(load).toBeDefined()

          const secondPromptPromise = secondConnection.connection.prompt({
            sessionId: session.sessionId,
            prompt: [{ type: 'text', text: promptText2 }],
          })

          const secondUpdate = await waitForMatchingUpdate(secondUpdates, (notification) => {
            return (
              notification.sessionId === session.sessionId &&
              notification.update.sessionUpdate === 'agent_message_chunk'
            )
          })

          const secondResponse = await secondPromptPromise

          expect(secondResponse.stopReason).toBe('end_turn')
          expect(secondUpdate.sessionId).toBe(session.sessionId)
          if (secondUpdate.update.sessionUpdate !== 'agent_message_chunk') {
            throw new Error(`unexpected update kind ${secondUpdate.update.sessionUpdate}`)
          }
          expect(secondUpdate.update.content.type).toBe('text')
          expect(secondUpdate.update.content.text).toContain('Hello')

          const sessionRow = await waitFor(
            () => db.collections.sessions.toArray.find((row) => row.sessionId === session.sessionId),
            5_000,
          )
          const secondPromptTurn = await waitFor(
            () =>
              db.collections.promptTurns.toArray.find(
                (turn) =>
                  turn.sessionId === session.sessionId &&
                  turn.state === 'completed' &&
                  turn.text === promptText2,
              ),
            5_000,
          )
          const secondChunk = await waitFor(
            () =>
              db.collections.chunks.toArray.find(
                (entry) =>
                  entry.promptTurnId === secondPromptTurn?.promptTurnId &&
                  entry.content.includes('Hello'),
              ),
            5_000,
          )

          expect(sessionRow).toBeDefined()
          expect(sessionRow?.supportsLoadSession).toBe(true)
          expect(secondPromptTurn).toBeDefined()
          expect(secondPromptTurn?.text).toBe(promptText2)
          expect(secondChunk).toBeDefined()
          expect(secondChunk?.type).toBe('text')
        } finally {
          await secondConnection.close()
        }
      } finally {
        db.close()
        await client.close()
        await deleteRuntime(harnessApiBase).catch(() => undefined)
      }
    },
    60_000,
  )
})

type HarnessRuntime = {
  runtimeKey: string
  runtimeId: string
  status: string
  acp: {
    url: string
    headers?: Record<string, string>
  }
  state: {
    url: string
    headers?: Record<string, string>
  }
}

async function waitForRuntimeReady(
  harnessApiBase: string,
  runtimeKey: string,
  timeoutMs = 20_000,
): Promise<HarnessRuntime> {
  return waitForDefined(async () => {
    const response = await fetchJson<{ runtime: HarnessRuntime | null }>(`${harnessApiBase}/runtime`)
    if (!response.runtime || response.runtime.runtimeKey !== runtimeKey) {
      return undefined
    }
    if (response.runtime.status === 'ready') {
      return response.runtime
    }
    return undefined
  }, timeoutMs, 100)
}

async function deleteRuntime(harnessApiBase: string): Promise<void> {
  await fetchJson<{ runtime: null }>(`${harnessApiBase}/runtime`, {
    method: 'DELETE',
  })
}

async function connectWithRetry(
  client: ReturnType<typeof createBrowserFirelineClient>,
  url: string,
  timeoutMs = 10_000,
): Promise<Awaited<ReturnType<typeof client.acp.connect>>> {
  const deadline = Date.now() + timeoutMs
  let lastError: unknown

  while (Date.now() < deadline) {
    try {
      return await client.acp.connect({ url })
    } catch (error) {
      lastError = error
      await sleep(100)
    }
  }

  throw lastError instanceof Error ? lastError : new Error('timed out waiting for ACP reconnect')
}

async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, {
    ...init,
    headers: {
      'content-type': 'application/json',
      ...(init?.headers ?? {}),
    },
  })

  if (!response.ok) {
    throw new Error(`request failed (${response.status} ${response.statusText})`)
  }

  return (await response.json()) as T
}

async function waitForMatchingUpdate(
  iterator: AsyncIterator<SessionNotification>,
  predicate: (update: SessionNotification) => boolean,
  timeoutMs = 10_000,
): Promise<SessionNotification> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const next = await Promise.race<
      IteratorResult<SessionNotification> | typeof UPDATE_TIMEOUT
    >([
      iterator.next(),
      sleep(timeoutMs).then(() => UPDATE_TIMEOUT),
    ])
    if (next === UPDATE_TIMEOUT) {
      break
    }
    if (next.done) {
      break
    }
    if (next.value && predicate(next.value)) {
      return next.value
    }
  }
  throw new Error('timed out waiting for matching ACP update')
}

async function waitFor<T>(getValue: () => T | undefined, timeoutMs: number): Promise<T | undefined> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const value = getValue()
    if (value !== undefined) {
      return value
    }
    await sleep(50)
  }
  return undefined
}

async function waitForDefined<T>(
  getValue: () => Promise<T | undefined>,
  timeoutMs: number,
  intervalMs: number,
): Promise<T> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const value = await getValue()
    if (value !== undefined) {
      return value
    }
    await sleep(intervalMs)
  }
  throw new Error('timed out waiting for value')
}

const UPDATE_TIMEOUT = Symbol('update-timeout')

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}
