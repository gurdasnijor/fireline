import { execFileSync } from 'node:child_process'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { randomUUID } from 'node:crypto'

import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import { createHostClient, type HostClient } from '../src/index.js'

const repoRoot = fileURLToPath(new URL('../../../', import.meta.url))
const firelineBin = join(repoRoot, 'target', 'debug', 'fireline')
const firelineTestyBin = join(repoRoot, 'target', 'debug', 'fireline-testy')

let tempRoot: string
let host: HostClient | undefined

describe('client.host', () => {
  beforeAll(async () => {
    execFileSync('cargo', ['build', '--quiet', '--bin', 'fireline', '--bin', 'fireline-testy'], {
      cwd: repoRoot,
      stdio: 'inherit',
    })
    tempRoot = await mkdtemp(join(tmpdir(), 'fireline-client-host-'))
  })

  afterAll(async () => {
    await host?.close()
    await rm(tempRoot, { recursive: true, force: true })
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
})
