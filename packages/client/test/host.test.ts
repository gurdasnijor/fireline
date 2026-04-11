import { execFileSync, spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { setTimeout as delay } from 'node:timers/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { randomUUID } from 'node:crypto'

import { parse as parseToml } from '@iarna/toml'
import { DurableStream } from '@durable-streams/client'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import { createHostClient, type HostClient, type RuntimeDescriptor } from '../src/index.js'

const repoRoot = fileURLToPath(new URL('../../../', import.meta.url))
const firelineBin = join(repoRoot, 'target', 'debug', 'fireline')
const firelineControlPlaneBin = join(repoRoot, 'target', 'debug', 'fireline-control-plane')
const firelineTestyBin = join(repoRoot, 'target', 'debug', 'fireline-testy')

let tempRoot: string
let host: HostClient | undefined
let controlPlane: ChildProcessWithoutNullStreams | undefined

describe('client.host', () => {
  beforeAll(async () => {
    execFileSync(
      'cargo',
      [
        'build',
        '--quiet',
        '-p',
        'fireline',
        '--bin',
        'fireline',
        '--bin',
        'fireline-testy',
        '-p',
        'fireline-control-plane',
        '--bin',
        'fireline-control-plane',
      ],
      {
        cwd: repoRoot,
        stdio: 'inherit',
      },
    )
    tempRoot = await mkdtemp(join(tmpdir(), 'fireline-client-host-'))
  })

  afterAll(async () => {
    await host?.close()
    await stopControlPlane()
    if (tempRoot) {
      await rm(tempRoot, { recursive: true, force: true })
    }
  })

  it('creates, lists, gets, stops, and deletes a local runtime', async () => {
    host = createHostClient({
      firelineBin,
      runtimeRegistryPath: join(tempRoot, 'runtimes.toml'),
      startupTimeoutMs: 20_000,
      stopTimeoutMs: 10_000,
    })

    const created = await host.create({
      provider: 'auto',
      host: '127.0.0.1',
      port: 0,
      name: `ts-host-${randomUUID()}`,
      agentCommand: [firelineTestyBin],
      peerDirectoryPath: join(tempRoot, 'peers.toml'),
    })

    expect(created.provider).toBe('local')
    expect(created.status).toBe('ready')
    expect(created.runtimeKey).toMatch(/^runtime:/)
    expect(created.runtimeId).toMatch(/^fireline:ts-host-/)
    expect(created.acp.url).toMatch(/^ws:\/\//)
    expect(created.state.url).toMatch(/^http:\/\//)

    const fetched = await host.get(created.runtimeKey)
    expect(fetched).toEqual(created)

    const listed = await host.list()
    expect(listed).toHaveLength(1)
    expect(listed[0]).toEqual(created)

    const stopped = await host.stop(created.runtimeKey)
    expect(stopped.status).toBe('stopped')

    const deleted = await host.delete(created.runtimeKey)
    expect(deleted?.runtimeKey).toBe(created.runtimeKey)
    expect(await host.get(created.runtimeKey)).toBeNull()
  })

  it('resume(sessionId) recreates a stopped runtime from its shared-stream runtime_spec', async () => {
    const streamHostRegistryPath = join(tempRoot, `resume-stream-host-${randomUUID()}.toml`)
    const streamHostName = `stream-host-${randomUUID()}`
    const streamHost = spawn(
      firelineBin,
      [
        '--host',
        '127.0.0.1',
        '--port',
        '0',
        '--name',
        streamHostName,
        '--runtime-registry-path',
        streamHostRegistryPath,
        '--',
        firelineTestyBin,
      ],
      {
        cwd: repoRoot,
        stdio: 'inherit',
      },
    )

    const controlPlanePort = 46000 + Math.floor(Math.random() * 1000)
    const controlPlaneUrl = `http://127.0.0.1:${controlPlanePort}`
    const sharedStreamName = `resume-test-${randomUUID()}`
    let localHost: HostClient | undefined
    let localControlPlane: ChildProcessWithoutNullStreams | undefined

    try {
      const streamBaseUrl = await waitForStreamHostBaseUrl(streamHostRegistryPath, streamHostName)
      const sharedStreamBaseUrl = streamBaseUrl
      const sharedStateUrl = `${sharedStreamBaseUrl}/${sharedStreamName}`

      localControlPlane = spawn(
        firelineControlPlaneBin,
        [
          '--host',
          '127.0.0.1',
          '--port',
          String(controlPlanePort),
          '--fireline-bin',
          firelineBin,
          '--runtime-registry-path',
          join(tempRoot, `resume-control-plane-runtimes-${randomUUID()}.toml`),
          '--peer-directory-path',
          join(tempRoot, `resume-control-plane-peers-${randomUUID()}.toml`),
          '--shared-stream-base-url',
          sharedStreamBaseUrl,
        ],
        {
          cwd: repoRoot,
          stdio: 'inherit',
        },
      )
      await waitForControlPlaneReady(controlPlaneUrl)

      localHost = createHostClient({
        controlPlaneUrl,
        sharedStateUrl,
      })

      const created = await localHost.create({
        provider: 'local',
        host: '127.0.0.1',
        port: 0,
        name: `resume-host-${randomUUID()}`,
        agentCommand: [firelineTestyBin],
        stateStream: sharedStreamName,
      })
      expect(created.status).toBe('ready')
      expect(created.runtimeKey).toMatch(/^runtime:/)

      const sessionId = `sess-${randomUUID()}`
      await appendSessionEnvelope(sharedStateUrl, sessionId, created)

      const stopped = await localHost.stop(created.runtimeKey)
      expect(stopped.status).toBe('stopped')

      const resumed = await localHost.resume(sessionId, { timeoutMs: 30_000 })
      expect(resumed.status).toBe('ready')
      expect(resumed.runtimeKey).toBe(created.runtimeKey)
      expect(resumed.runtimeId).not.toBe(created.runtimeId)
      expect(resumed.runtimeId).toMatch(/^fireline:resume-host-/)
    } finally {
      if (localHost) {
        try {
          await localHost.close()
        } catch {
          // best-effort client teardown
        }
      }
      if (localControlPlane) {
        if (localControlPlane.exitCode === null && localControlPlane.signalCode === null) {
          localControlPlane.kill('SIGTERM')
          await new Promise<void>((resolve) => {
            localControlPlane?.once('exit', () => resolve())
            setTimeout(resolve, 5_000)
          })
        }
      }
      if (streamHost.exitCode === null && streamHost.signalCode === null) {
        streamHost.kill('SIGINT')
        await new Promise<void>((resolve) => {
          streamHost.once('exit', () => resolve())
          setTimeout(resolve, 5_000)
        })
      }
    }
  }, 120_000)

  it('creates, lists, gets, stops, and deletes a runtime through the control plane', async () => {
    const controlPlanePort = 45000 + Math.floor(Math.random() * 1000)
    const controlPlaneUrl = `http://127.0.0.1:${controlPlanePort}`
    await stopControlPlane()
    controlPlane = spawn(
      firelineControlPlaneBin,
      [
        '--host',
        '127.0.0.1',
        '--port',
        String(controlPlanePort),
        '--fireline-bin',
        firelineBin,
        '--runtime-registry-path',
        join(tempRoot, 'control-plane-runtimes.toml'),
        '--peer-directory-path',
        join(tempRoot, 'control-plane-peers.toml'),
      ],
      {
        cwd: repoRoot,
        stdio: 'inherit',
      },
    )
    await waitForControlPlaneReady(controlPlaneUrl)

    host = createHostClient({
      controlPlaneUrl,
    })

    const created = await host.create({
      provider: 'local',
      host: '127.0.0.1',
      port: 0,
      name: `cp-host-${randomUUID()}`,
      agentCommand: [firelineTestyBin],
    })

    expect(created.provider).toBe('local')
    expect(created.status).toBe('ready')
    expect(created.runtimeKey).toMatch(/^runtime:/)
    expect(created.runtimeId).toMatch(/^fireline:cp-host-/)
    expect(created.acp.url).toMatch(/^ws:\/\//)
    expect(created.state.url).toMatch(/^http:\/\//)

    const fetched = await host.get(created.runtimeKey)
    expect(fetched).toEqual(created)

    const listed = await host.list()
    expect(listed).toHaveLength(1)
    expect(listed[0]).toEqual(created)

    const stopped = await host.stop(created.runtimeKey)
    expect(stopped.status).toBe('stopped')

    const deleted = await host.delete(created.runtimeKey)
    expect(deleted?.runtimeKey).toBe(created.runtimeKey)
    expect(await host.get(created.runtimeKey)).toBeNull()
  })
})

async function waitForControlPlaneReady(controlPlaneUrl: string): Promise<void> {
  const deadline = Date.now() + 10_000
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${controlPlaneUrl}/healthz`)
      if (response.ok) {
        return
      }
    } catch {
      // Keep polling until the server is listening.
    }

    await new Promise((resolve) => setTimeout(resolve, 100))
  }

  throw new Error('timed out waiting for control plane to become ready')
}

async function waitForStreamHostBaseUrl(
  runtimeRegistryPath: string,
  runtimeName: string,
): Promise<string> {
  const deadline = Date.now() + 20_000
  const idPrefix = `fireline:${runtimeName}:`
  while (Date.now() < deadline) {
    try {
      const raw = await readFile(runtimeRegistryPath, 'utf8')
      if (raw.trim()) {
        const parsed = parseToml(raw) as { runtimes?: Array<{ runtimeId?: string; status?: string; state?: { url?: string } }> }
        const runtimes = parsed.runtimes ?? []
        const hit = runtimes.find(
          (entry) => entry.runtimeId?.startsWith(idPrefix) && entry.status === 'ready' && entry.state?.url,
        )
        if (hit?.state?.url) {
          const fullUrl = hit.state.url
          const marker = '/v1/stream/'
          const idx = fullUrl.indexOf(marker)
          if (idx < 0) {
            throw new Error(`unexpected stream url format: ${fullUrl}`)
          }
          return fullUrl.slice(0, idx + marker.length - 1)
        }
      }
    } catch (error) {
      if (!(typeof error === 'object' && error !== null && 'code' in error && (error as { code?: string }).code === 'ENOENT')) {
        throw error
      }
    }
    await delay(100)
  }
  throw new Error(`timed out waiting for stream host '${runtimeName}' to register`)
}

async function appendSessionEnvelope(
  sharedStateUrl: string,
  sessionId: string,
  runtime: RuntimeDescriptor,
): Promise<void> {
  const stream = new DurableStream({
    url: sharedStateUrl,
    contentType: 'application/json',
  })
  try {
    await stream.create({ contentType: 'application/json' })
  } catch (error) {
    // The stream may already exist because the control plane's first
    // runtime_spec emit auto-created it. Ignore "conflict exists" and
    // proceed to append.
    const code = (error as { code?: string } | null)?.code
    if (code !== 'CONFLICT_EXISTS') {
      throw error
    }
  }

  const now = Date.now()
  const envelope = {
    type: 'session',
    key: sessionId,
    headers: { operation: 'insert' },
    value: {
      sessionId,
      runtimeKey: runtime.runtimeKey,
      runtimeId: runtime.runtimeId,
      nodeId: runtime.nodeId,
      logicalConnectionId: `conn:${sessionId}`,
      state: 'active',
      supportsLoadSession: false,
      createdAt: now,
      updatedAt: now,
      lastSeenAt: now,
    },
  }
  await stream.append(JSON.stringify(envelope))
}

async function stopControlPlane(): Promise<void> {
  if (!controlPlane) {
    return
  }

  if (controlPlane.exitCode !== null || controlPlane.signalCode !== null) {
    controlPlane = undefined
    return
  }

  controlPlane.kill('SIGTERM')
  await new Promise<void>((resolve) => {
    controlPlane?.once('exit', () => resolve())
    setTimeout(resolve, 5_000)
  })
  controlPlane = undefined
}
