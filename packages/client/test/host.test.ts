import { execFileSync, spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { randomUUID } from 'node:crypto'

import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import { createHostClient, type HostClient } from '../src/index.js'

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
    expect(created.acpUrl).toMatch(/^ws:\/\//)
    expect(created.stateStreamUrl).toMatch(/^http:\/\//)

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
    expect(created.acpUrl).toMatch(/^ws:\/\//)
    expect(created.stateStreamUrl).toMatch(/^http:\/\//)

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
