import { execFileSync } from 'node:child_process'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { randomUUID } from 'node:crypto'

import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { SessionNotification } from '@agentclientprotocol/sdk'

import { createFirelineClient, type FirelineClient } from '../src/index.js'

const repoRoot = fileURLToPath(new URL('../../../', import.meta.url))
const firelineBin = join(repoRoot, 'target', 'debug', 'fireline')
const firelineTestyBin = join(repoRoot, 'target', 'debug', 'fireline-testy')

let tempRoot: string
let client: FirelineClient | undefined

describe('client.acp.connect', () => {
  beforeAll(async () => {
    execFileSync('cargo', ['build', '--quiet', '--bin', 'fireline', '--bin', 'fireline-testy'], {
      cwd: repoRoot,
      stdio: 'inherit',
    })
    tempRoot = await mkdtemp(join(tmpdir(), 'fireline-client-acp-'))
  }, 30_000)

  afterAll(async () => {
    await client?.close()
    await rm(tempRoot, { recursive: true, force: true })
  })

  it(
    'connects to a hosted runtime, prompts over ACP, and observes durable state',
    async () => {
      client = createFirelineClient({
        host: {
          firelineBin,
          runtimeRegistryPath: join(tempRoot, 'runtimes.toml'),
          startupTimeoutMs: 20_000,
          stopTimeoutMs: 10_000,
        },
      })

      const runtime = await client.host.create({
        provider: 'local',
        host: '127.0.0.1',
        port: 0,
        name: `ts-acp-${randomUUID()}`,
        agentCommand: [firelineTestyBin],
        peerDirectoryPath: join(tempRoot, 'peers.toml'),
      })

      const db = client.state.open({ stateStreamUrl: runtime.state.url })
      await db.preload()

      const acp = await client.acp.connect({ url: runtime.acp.url })
      const updates = acp.updates()[Symbol.asyncIterator]()
      const conn = acp.connection

      try {
        const init = await acp.initialize()
        expect(init.protocolVersion).toBe(1)

        const session = await conn.newSession({
          cwd: repoRoot,
          mcpServers: [],
        })

        const promptPromise = conn.prompt({
          sessionId: session.sessionId,
          prompt: [
            {
              type: 'text',
              text: 'hello from TypeScript',
            },
          ],
        })

        const update = await waitForMatchingUpdate(updates, (notification) => {
          return (
            notification.sessionId === session.sessionId &&
            notification.update.sessionUpdate === 'agent_message_chunk'
          )
        })

        const response = await promptPromise

        expect(response.stopReason).toBe('end_turn')
        expect(update.sessionId).toBe(session.sessionId)
        if (update.update.sessionUpdate !== 'agent_message_chunk') {
          throw new Error(`unexpected update kind ${update.update.sessionUpdate}`)
        }
        expect(update.update.content.type).toBe('text')
        expect(update.update.content.text).toContain('Hello')

        const promptTurn = await waitFor(
          () =>
            db.collections.promptTurns.toArray.find(
              (turn) => turn.sessionId === session.sessionId && turn.state === 'completed',
            ),
          5_000,
        )

        expect(promptTurn).toBeDefined()
        expect(promptTurn?.text).toBe('hello from TypeScript')

        const chunk = await waitFor(
          () =>
            db.collections.chunks.toArray.find(
              (entry) =>
                entry.promptTurnId === promptTurn?.promptTurnId && entry.content.includes('Hello'),
            ),
          5_000,
        )

        expect(chunk).toBeDefined()
        expect(chunk?.type).toBe('text')
      } finally {
        await acp.close()
        db.close()
      }
    },
    20_000,
  )
})

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

const UPDATE_TIMEOUT = Symbol('update-timeout')

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}
