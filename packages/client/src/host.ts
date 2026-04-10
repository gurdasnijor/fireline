import { randomUUID } from 'node:crypto'
import { spawn, type ChildProcessByStdio } from 'node:child_process'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { homedir, platform } from 'node:os'
import { dirname, join } from 'node:path'
import type { Readable } from 'node:stream'
import { setTimeout as delay } from 'node:timers/promises'

import { parse, stringify } from '@iarna/toml'
import {
  createCatalogClient,
  type CatalogClientOptions,
  type RuntimeAgentSpec,
} from './catalog.js'
import type { TopologySpec } from './topology.js'

export type RuntimeProviderRequest = 'auto' | 'local'
export type RuntimeProviderKind = 'local'

export type RuntimeStatus =
  | 'starting'
  | 'ready'
  | 'busy'
  | 'idle'
  | 'stale'
  | 'broken'
  | 'stopped'

export interface RuntimeDescriptor {
  runtimeKey: string
  runtimeId: string
  nodeId: string
  provider: RuntimeProviderKind
  providerInstanceId: string
  status: RuntimeStatus
  acpUrl: string
  stateStreamUrl: string
  helperApiBaseUrl?: string
  createdAtMs: number
  updatedAtMs: number
}

export interface HostClientOptions {
  controlPlaneUrl?: string
  controlPlaneToken?: string
  firelineBin?: string
  runtimeRegistryPath?: string
  pollIntervalMs?: number
  startupTimeoutMs?: number
  stopTimeoutMs?: number
  catalog?: CatalogClientOptions
}

interface CreateRuntimeSpecBase {
  provider?: RuntimeProviderRequest
  host?: string
  port?: number
  name?: string
  stateStream?: string
  peerDirectoryPath?: string
  topology?: TopologySpec
}

export type CreateRuntimeSpec =
  | (CreateRuntimeSpecBase & {
      agentCommand: string[]
      agent?: never
    })
  | (CreateRuntimeSpecBase & {
      agent: RuntimeAgentSpec
      agentCommand?: never
    })


export interface HostClient {
  create(spec: CreateRuntimeSpec): Promise<RuntimeDescriptor>
  get(runtimeKey: string): Promise<RuntimeDescriptor | null>
  list(): Promise<RuntimeDescriptor[]>
  stop(runtimeKey: string): Promise<RuntimeDescriptor>
  delete(runtimeKey: string): Promise<RuntimeDescriptor | null>
  close(): Promise<void>
}

interface RuntimeRegistryFile {
  runtimes?: RuntimeDescriptor[]
}

type FirelineChildProcess = ChildProcessByStdio<null, Readable, Readable>

interface OwnedRuntime {
  child: FirelineChildProcess
  logs: string[]
  exited: boolean
}

const DEFAULT_POLL_INTERVAL_MS = 100
const DEFAULT_STARTUP_TIMEOUT_MS = 20_000
const DEFAULT_STOP_TIMEOUT_MS = 10_000
const LOG_RING_SIZE = 32

export function createHostClient(options: HostClientOptions = {}): HostClient {
  if (options.controlPlaneUrl) {
    return createControlPlaneHostClient(options)
  }

  const runtimeRegistryPath = options.runtimeRegistryPath ?? defaultRuntimeRegistryPath()
  const firelineBin = options.firelineBin ?? 'fireline'
  const pollIntervalMs = options.pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS
  const startupTimeoutMs = options.startupTimeoutMs ?? DEFAULT_STARTUP_TIMEOUT_MS
  const stopTimeoutMs = options.stopTimeoutMs ?? DEFAULT_STOP_TIMEOUT_MS
  const catalog = createCatalogClient(options.catalog)
  const owned = new Map<string, OwnedRuntime>()

  const hostClient: HostClient = {
    async create(spec) {
      const runtimeName = spec.name ?? `fireline-ts-${randomUUID()}`
      const agentCommand = await resolveCreateRuntimeAgentCommand(spec, catalog)
      const beforeKeys = new Set((await listRuntimes(runtimeRegistryPath)).map((runtime) => runtime.runtimeKey))
      const startedAt = Date.now()
      const child = spawnFireline({
        firelineBin,
        runtimeRegistryPath,
        host: spec.host ?? '127.0.0.1',
        port: spec.port ?? 0,
        name: runtimeName,
        stateStream: spec.stateStream,
        agentCommand,
        peerDirectoryPath: spec.peerDirectoryPath,
        topology: spec.topology,
      })

      const owner: OwnedRuntime = {
        child,
        logs: [],
        exited: false,
      }
      wireProcessLogs(owner)

      try {
        const descriptor = await waitForRuntimeReady({
          runtimeRegistryPath,
          runtimeName,
          beforeKeys,
          startedAt,
          owner,
          pollIntervalMs,
          startupTimeoutMs,
        })
        owned.set(descriptor.runtimeKey, owner)
        owner.child.once('exit', () => {
          owner.exited = true
          owned.delete(descriptor.runtimeKey)
        })
        return descriptor
      } catch (error) {
        if (!owner.exited) {
          owner.child.kill('SIGINT')
          await waitForChildExit(owner.child, stopTimeoutMs).catch(() => undefined)
        }
        throw error
      }
    },

    async get(runtimeKey) {
      const runtimes = await listRuntimes(runtimeRegistryPath)
      return runtimes.find((runtime) => runtime.runtimeKey === runtimeKey) ?? null
    },

    async list() {
      return listRuntimes(runtimeRegistryPath)
    },

    async stop(runtimeKey) {
      const owner = owned.get(runtimeKey)
      if (!owner) {
        throw new Error(
          `runtime '${runtimeKey}' is not owned by this host client; local stop requires the original process handle`,
        )
      }

      if (!owner.exited) {
        owner.child.kill('SIGINT')
        await waitForChildExit(owner.child, stopTimeoutMs)
      }
      owned.delete(runtimeKey)

      return waitForRuntimeStopped(runtimeRegistryPath, runtimeKey, pollIntervalMs, stopTimeoutMs)
    },

    async delete(runtimeKey) {
      const existing = await hostClient.get(runtimeKey)
      if (!existing) {
        return null
      }

      if (existing.status !== 'stopped') {
        await hostClient.stop(runtimeKey)
      }

      const removed = await removeRuntime(runtimeRegistryPath, runtimeKey)
      owned.delete(runtimeKey)
      return removed
    },

    async close() {
      const runtimeKeys = [...owned.keys()]
      for (const runtimeKey of runtimeKeys) {
        try {
          await hostClient.stop(runtimeKey)
        } catch {
          // Best-effort shutdown for owned local runtimes.
        }
      }
    },
  }

  return hostClient
}

function createControlPlaneHostClient(options: HostClientOptions): HostClient {
  const controlPlaneUrl = options.controlPlaneUrl
  if (!controlPlaneUrl) {
    throw new Error('controlPlaneUrl is required for control-plane host mode')
  }

  const catalog = createCatalogClient(options.catalog)
  const baseUrl = controlPlaneUrl.replace(/\/$/, '')

  return {
    async create(spec) {
      const runtimeName = spec.name ?? `fireline-ts-${randomUUID()}`
      const agentCommand = await resolveCreateRuntimeAgentCommand(spec, catalog)
      return requestControlPlane<RuntimeDescriptor>(baseUrl, '/v1/runtimes', {
        token: options.controlPlaneToken,
        method: 'POST',
        body: JSON.stringify({
          provider: spec.provider ?? 'local',
          host: spec.host ?? '127.0.0.1',
          port: spec.port ?? 0,
          name: runtimeName,
          agentCommand,
          stateStream: spec.stateStream,
          peerDirectoryPath: spec.peerDirectoryPath,
          topology: spec.topology ?? { components: [] },
        }),
      })
    },

    async get(runtimeKey) {
      return requestControlPlane<RuntimeDescriptor | null>(
        baseUrl,
        `/v1/runtimes/${encodeURIComponent(runtimeKey)}`,
        {
          token: options.controlPlaneToken,
          allowNotFound: true,
        },
      )
    },

    async list() {
      return requestControlPlane<RuntimeDescriptor[]>(baseUrl, '/v1/runtimes', {
        token: options.controlPlaneToken,
      })
    },

    async stop(runtimeKey) {
      return requestControlPlane<RuntimeDescriptor>(
        baseUrl,
        `/v1/runtimes/${encodeURIComponent(runtimeKey)}/stop`,
        {
          token: options.controlPlaneToken,
          method: 'POST',
        },
      )
    },

    async delete(runtimeKey) {
      return requestControlPlane<RuntimeDescriptor | null>(
        baseUrl,
        `/v1/runtimes/${encodeURIComponent(runtimeKey)}`,
        {
          token: options.controlPlaneToken,
          method: 'DELETE',
          allowNotFound: true,
        },
      )
    },

    async close() {
      // Control-plane lifecycle is owned by the server process.
    },
  }
}

async function resolveCreateRuntimeAgentCommand(
  spec: CreateRuntimeSpec,
  catalog: ReturnType<typeof createCatalogClient>,
): Promise<string[]> {
  if ('agentCommand' in spec && Array.isArray(spec.agentCommand)) {
    return [...spec.agentCommand]
  }

  const agent = spec.agent
  if (!agent) {
    throw new Error('runtime create spec must include either agentCommand or agent')
  }

  if (agent.source === 'manual') {
    return [...agent.command]
  }

  const resolved = await catalog.resolveAgent(agent.agentId, {
    preferredKinds: agent.preferredKinds,
  })
  return resolved.command
}

export function defaultRuntimeRegistryPath(): string {
  const home = homedir()
  switch (platform()) {
    case 'darwin':
      return join(home, 'Library', 'Application Support', 'fireline', 'runtimes.toml')
    case 'win32':
      return join(process.env.LOCALAPPDATA ?? join(home, 'AppData', 'Local'), 'fireline', 'runtimes.toml')
    default:
      return join(process.env.XDG_DATA_HOME ?? join(home, '.local', 'share'), 'fireline', 'runtimes.toml')
  }
}

async function listRuntimes(runtimeRegistryPath: string): Promise<RuntimeDescriptor[]> {
  const file = await readRuntimeRegistry(runtimeRegistryPath)
  return [...(file.runtimes ?? [])]
}

async function removeRuntime(
  runtimeRegistryPath: string,
  runtimeKey: string,
): Promise<RuntimeDescriptor | null> {
  const file = await readRuntimeRegistry(runtimeRegistryPath)
  const runtimes = file.runtimes ?? []
  const removed = runtimes.find((runtime) => runtime.runtimeKey === runtimeKey) ?? null
  await writeRuntimeRegistry(runtimeRegistryPath, {
    runtimes: runtimes.filter((runtime) => runtime.runtimeKey !== runtimeKey),
  })
  return removed
}

async function readRuntimeRegistry(runtimeRegistryPath: string): Promise<RuntimeRegistryFile> {
  try {
    await mkdir(dirname(runtimeRegistryPath), { recursive: true })
    const raw = await readFile(runtimeRegistryPath, 'utf8')
    if (!raw.trim()) {
      return { runtimes: [] }
    }
    const parsed = parse(raw) as RuntimeRegistryFile
    return {
      runtimes: Array.isArray(parsed.runtimes) ? parsed.runtimes : [],
    }
  } catch (error) {
    if (isMissingFileError(error)) {
      return { runtimes: [] }
    }
    throw error
  }
}

async function writeRuntimeRegistry(runtimeRegistryPath: string, file: RuntimeRegistryFile): Promise<void> {
  await mkdir(dirname(runtimeRegistryPath), { recursive: true })
  const stringifyRegistry = stringify as unknown as (registry: RuntimeRegistryFile) => string
  await writeFile(runtimeRegistryPath, stringifyRegistry({ runtimes: file.runtimes ?? [] }), 'utf8')
}

function spawnFireline(spec: {
  firelineBin: string
  runtimeRegistryPath: string
  host: string
  port: number
  name: string
  stateStream?: string
  agentCommand: string[]
  peerDirectoryPath?: string
  topology?: TopologySpec
}): FirelineChildProcess {
  const args = [
    '--host',
    spec.host,
    '--port',
    String(spec.port),
    '--name',
    spec.name,
    '--runtime-registry-path',
    spec.runtimeRegistryPath,
  ]
  if (spec.peerDirectoryPath) {
    args.push('--peer-directory-path', spec.peerDirectoryPath)
  }
  if (spec.stateStream) {
    args.push('--state-stream', spec.stateStream)
  }
  if (spec.topology) {
    args.push('--topology-json', JSON.stringify(spec.topology))
  }
  args.push('--', ...spec.agentCommand)

  return spawn(spec.firelineBin, args, {
    stdio: ['ignore', 'pipe', 'pipe'],
    env: process.env,
  })
}

function wireProcessLogs(owner: OwnedRuntime): void {
  const push = (chunk: Buffer) => {
    owner.logs.push(chunk.toString('utf8').trim())
    if (owner.logs.length > LOG_RING_SIZE) {
      owner.logs.splice(0, owner.logs.length - LOG_RING_SIZE)
    }
  }

  owner.child.stdout.on('data', push)
  owner.child.stderr.on('data', push)
  owner.child.once('error', (error) => {
    owner.logs.push(error.message)
    owner.exited = true
  })
  owner.child.once('exit', () => {
    owner.exited = true
  })
}

async function waitForRuntimeReady(options: {
  runtimeRegistryPath: string
  runtimeName: string
  beforeKeys: Set<string>
  startedAt: number
  owner: OwnedRuntime
  pollIntervalMs: number
  startupTimeoutMs: number
}): Promise<RuntimeDescriptor> {
  const deadline = Date.now() + options.startupTimeoutMs
  const runtimeIdPrefix = `fireline:${options.runtimeName}:`

  while (Date.now() < deadline) {
    const runtimes = await listRuntimes(options.runtimeRegistryPath)
    const candidates = runtimes
      .filter(
        (runtime) =>
          !options.beforeKeys.has(runtime.runtimeKey) &&
          runtime.runtimeId.startsWith(runtimeIdPrefix) &&
          runtime.createdAtMs >= options.startedAt - 5_000,
      )
      .sort((left, right) => right.updatedAtMs - left.updatedAtMs)

    const ready = candidates.find((runtime) => runtime.status === 'ready')
    if (ready) {
      return ready
    }

    if (options.owner.exited) {
      throw new Error(`fireline process exited before runtime became ready:\n${formatLogs(options.owner.logs)}`)
    }

    await delay(options.pollIntervalMs)
  }

  throw new Error(`timed out waiting for runtime '${options.runtimeName}' to become ready`)
}

async function waitForRuntimeStopped(
  runtimeRegistryPath: string,
  runtimeKey: string,
  pollIntervalMs: number,
  timeoutMs: number,
): Promise<RuntimeDescriptor> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const runtime = (await listRuntimes(runtimeRegistryPath)).find((entry) => entry.runtimeKey === runtimeKey)
    if (runtime?.status === 'stopped') {
      return runtime
    }
    await delay(pollIntervalMs)
  }
  throw new Error(`timed out waiting for runtime '${runtimeKey}' to stop`)
}

async function waitForChildExit(
  child: FirelineChildProcess,
  timeoutMs: number,
): Promise<{ code: number | null; signal: NodeJS.Signals | null }> {
  if (child.exitCode !== null || child.signalCode !== null) {
    return {
      code: child.exitCode,
      signal: child.signalCode,
    }
  }

  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      child.kill('SIGKILL')
      reject(new Error(`timed out waiting for fireline process ${child.pid} to exit`))
    }, timeoutMs)

    child.once('exit', (code, signal) => {
      clearTimeout(timeout)
      resolve({ code, signal })
    })
  })
}

function formatLogs(logs: string[]): string {
  return logs.filter(Boolean).join('\n')
}

function isMissingFileError(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    (error as { code?: string }).code === 'ENOENT'
  )
}

async function requestControlPlane<T>(
  baseUrl: string,
  path: string,
  options: {
    token?: string
    method?: string
    body?: string
    allowNotFound?: boolean
  } = {},
): Promise<T> {
  const response = await fetch(`${baseUrl}${path}`, {
    method: options.method ?? 'GET',
    headers: {
      accept: 'application/json',
      ...(options.body ? { 'content-type': 'application/json' } : {}),
      ...(options.token ? { authorization: `Bearer ${options.token}` } : {}),
    },
    body: options.body,
  })

  if (response.status === 404 && options.allowNotFound) {
    return null as T
  }

  if (!response.ok) {
    const message = await readControlPlaneError(response)
    throw new Error(`${response.status} ${response.statusText}: ${message}`)
  }

  return (await response.json()) as T
}

async function readControlPlaneError(response: Response): Promise<string> {
  try {
    const payload = (await response.json()) as { error?: string }
    return payload.error ?? response.statusText
  } catch {
    return response.statusText
  }
}
