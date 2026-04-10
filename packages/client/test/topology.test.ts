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
const firelineTestyPromptBin = join(repoRoot, 'target', 'debug', 'fireline-testy-prompt')

let tempRoot: string
let client: FirelineClient | undefined

describe('runtime topology', () => {
  beforeAll(async () => {
    execFileSync(
      'cargo',
      ['build', '--quiet', '--bin', 'fireline', '--bin', 'fireline-testy', '--bin', 'fireline-testy-prompt'],
      {
        cwd: repoRoot,
        stdio: 'inherit',
      },
    )
    tempRoot = await mkdtemp(join(tmpdir(), 'fireline-client-topology-'))
  }, 30_000)

  afterAll(async () => {
    await client?.close()
    if (tempRoot) {
      await rm(tempRoot, { recursive: true, force: true })
    }
  })

  it('writes audit records while injecting context into the downstream agent prompt', async () => {
    client = createFirelineClient({
      host: {
        firelineBin,
        runtimeRegistryPath: join(tempRoot, 'runtimes.toml'),
        startupTimeoutMs: 20_000,
        stopTimeoutMs: 10_000,
      },
    })

    const auditStreamName = `fireline-audit-${randomUUID()}`
    const runtime = await withStepTimeout(
      'start runtime with audit/context topology',
      client.host.create({
        provider: 'local',
        host: '127.0.0.1',
        port: 0,
        name: `ts-topology-${randomUUID()}`,
        agentCommand: [firelineTestyPromptBin],
        peerDirectoryPath: join(tempRoot, 'peers-a.toml'),
        topology: client.topology
          .builder()
          .audit({ streamName: auditStreamName, includeMethods: ['session/prompt'] })
          .contextInjection({ prependText: 'Injected runtime context' })
          .build(),
      }),
      8_000,
    )

    const acp = await client.acp.connect({ url: runtime.acp.url })
    const updates = acp.updates()[Symbol.asyncIterator]()

    try {
      await withStepTimeout('initialize ACP connection', acp.initialize())
      const session = await withStepTimeout(
        'create ACP session',
        acp.connection.newSession({
          cwd: repoRoot,
          mcpServers: [],
        }),
      )

      const promptPromise = withStepTimeout(
        'complete prompt request',
        acp.connection.prompt({
          sessionId: session.sessionId,
          prompt: [
            {
              type: 'text',
              text: 'Original prompt',
            },
          ],
        }),
      )

      const update = await withStepTimeout(
        'receive injected prompt update',
        waitForMatchingUpdate(updates, (notification) => {
          return (
            notification.sessionId === session.sessionId &&
            notification.update.sessionUpdate === 'agent_message_chunk'
          )
        }),
      )
      await promptPromise

      if (update.update.sessionUpdate !== 'agent_message_chunk') {
        throw new Error(`unexpected update kind ${update.update.sessionUpdate}`)
      }
      expect(update.update.content.type).toBe('text')
      expect(update.update.content.text).toContain('Injected runtime context')
      expect(update.update.content.text).toContain('Original prompt')

      const auditStreamUrl = deriveSiblingStreamUrl(runtime.state.url, auditStreamName)
      const auditBody = await withStepTimeout(
        'read audit stream catch-up',
        readStreamUntil(auditStreamUrl, '"method":"session/prompt"'),
      )
      expect(auditBody).toContain('"direction":"request"')
      expect(auditBody).toContain('"direction":"response"')
      expect(auditBody).not.toContain('"method":"session/new"')
    } finally {
      await closeConnection(acp)
    }
  }, 20_000)

  it('injects peer MCP through runtime topology', async () => {
    client = createFirelineClient({
      host: {
        firelineBin,
        runtimeRegistryPath: join(tempRoot, 'runtimes-b.toml'),
        startupTimeoutMs: 20_000,
        stopTimeoutMs: 10_000,
      },
    })

    const runtime = await withStepTimeout(
      'start runtime with peer topology',
      client.host.create({
        provider: 'local',
        host: '127.0.0.1',
        port: 0,
        name: `ts-peer-topology-${randomUUID()}`,
        agentCommand: [firelineTestyBin],
        peerDirectoryPath: join(tempRoot, 'peers-b.toml'),
        topology: client.topology.builder().peerMcp().build(),
      }),
      8_000,
    )

    const acp = await client.acp.connect({ url: runtime.acp.url })
    const updates = acp.updates()[Symbol.asyncIterator]()

    try {
      await withStepTimeout('initialize ACP connection', acp.initialize())
      const session = await withStepTimeout(
        'create ACP session',
        acp.connection.newSession({
          cwd: repoRoot,
          mcpServers: [],
        }),
      )

      const listToolsPrompt = JSON.stringify({
        command: 'list_tools',
        server: 'fireline-peer',
      })

      const promptPromise = withStepTimeout(
        'complete prompt request',
        acp.connection.prompt({
          sessionId: session.sessionId,
          prompt: [
            {
              type: 'text',
              text: listToolsPrompt,
            },
          ],
        }),
      )

      const update = await withStepTimeout(
        'receive peer MCP update',
        waitForMatchingUpdate(updates, (notification) => {
          return (
            notification.sessionId === session.sessionId &&
            notification.update.sessionUpdate === 'agent_message_chunk'
          )
        }),
      )
      await promptPromise

      if (update.update.sessionUpdate !== 'agent_message_chunk') {
        throw new Error(`unexpected update kind ${update.update.sessionUpdate}`)
      }
      expect(update.update.content.type).toBe('text')
      expect(update.update.content.text).toContain('list_peers')
      expect(update.update.content.text).toContain('prompt_peer')
    } finally {
      await closeConnection(acp)
    }
  }, 20_000)
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
    if (next === UPDATE_TIMEOUT || next.done) {
      break
    }
    if (predicate(next.value)) {
      return next.value
    }
  }
  throw new Error('timed out waiting for matching ACP update')
}

function deriveSiblingStreamUrl(stateStreamUrl: string, streamName: string): string {
  const url = new URL(stateStreamUrl)
  const path = url.pathname
  const idx = path.lastIndexOf('/')
  url.pathname = `${path.slice(0, idx)}/${streamName}`
  return url.toString()
}

async function readStreamUntil(url: string, expected: string, timeoutMs = 10_000): Promise<string> {
  const deadline = Date.now() + timeoutMs
  let lastBody = ''

  while (Date.now() < deadline) {
    const response = await fetch(readFromBeginning(url))
    if (!response.ok) {
      throw new Error(`failed to read stream ${url}: ${response.status} ${response.statusText}`)
    }
    lastBody = await response.text()
    if (lastBody.includes(expected)) {
      return lastBody
    }
    await sleep(250)
  }

  throw new Error(`timed out waiting for '${expected}' in stream ${url}\n${lastBody}`)
}

function readFromBeginning(url: string): string {
  const streamUrl = new URL(url)
  streamUrl.searchParams.set('offset', '-1')
  return streamUrl.toString()
}

const UPDATE_TIMEOUT = Symbol('update-timeout')

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}

async function closeConnection(connection: { close(): Promise<void> }): Promise<void> {
  await Promise.race([connection.close(), sleep(1_000)])
}

async function withStepTimeout<T>(
  label: string,
  promise: Promise<T>,
  timeoutMs = 5_000,
): Promise<T> {
  return Promise.race([
    promise,
    sleep(timeoutMs).then(() => {
      throw new Error(`timed out while waiting to ${label}`)
    }),
  ])
}
