/**
 * Fireline-specific Host satisfier that wraps the control-plane HTTP surface
 * described in `docs/proposals/client-primitives.md` and grounded in
 * `docs/proposals/runtime-host-split.md` §7. A Host provisions a runtime
 * instance and returns a handle carrying the runtime's ACP + state-stream
 * endpoints; session lifecycle lives on the ACP data plane inside the
 * provisioned runtime, not on this interface.
 */
import type {
  Host,
  HostHandle,
  HostStatus,
  ProvisionSpec,
  WakeOutcome,
} from '../host/index.js'

export interface FirelineHostOptions {
  readonly controlPlaneUrl: string
  readonly controlPlaneToken?: string
  readonly sharedStateUrl: string
}

type FirelineRuntimeStatus =
  | 'starting'
  | 'ready'
  | 'busy'
  | 'idle'
  | 'stale'
  | 'broken'
  | 'stopped'

type FirelineEndpointWire = {
  readonly url: string
  readonly headers?: Readonly<Record<string, string>>
}

type FirelineRuntimeDescriptor = {
  readonly runtimeKey: string
  readonly status: FirelineRuntimeStatus
  readonly acp?: FirelineEndpointWire
  readonly state?: FirelineEndpointWire
}

const DEFAULT_POLL_INTERVAL_MS = 100
const DEFAULT_STARTUP_TIMEOUT_MS = 20_000

export function createFirelineHost(opts: FirelineHostOptions): Host {
  const baseUrl = opts.controlPlaneUrl.replace(/\/$/, '')
  const token = opts.controlPlaneToken
  const sharedStateUrl = opts.sharedStateUrl

  return {
    async provision(spec) {
      const runtimeName = readMetadataString(spec.metadata, 'name') ?? `fireline-ts-${crypto.randomUUID()}`
      const stateStream = readMetadataString(spec.metadata, 'stateStream')
      const explicitPort = readMetadataNumber(spec.metadata, 'port')

      const descriptor = await requestControlPlane<FirelineRuntimeDescriptor>(baseUrl, '/v1/runtimes', {
        token,
        method: 'POST',
        body: JSON.stringify({
          provider: 'local',
          host: '127.0.0.1',
          port: explicitPort ?? 0,
          name: runtimeName,
          agentCommand: spec.agentCommand ?? [],
          topology: spec.topology ?? { components: [] },
          resources: spec.resources ?? [],
          stateStream,
        }),
      })

      const ready = await waitForRuntimeReady(baseUrl, token, descriptor.runtimeKey, DEFAULT_STARTUP_TIMEOUT_MS)
      return {
        id: ready.runtimeKey,
        kind: 'fireline',
        acp: ready.acp ?? { url: '' },
        state: ready.state ?? { url: sharedStateUrl },
      }
    },

    async wake(handle) {
      const descriptor = await getRuntime(baseUrl, token, handle.id)
      if (!descriptor) {
        return blockedWakeOutcome()
      }

      if (descriptor.status === 'ready') {
        return { kind: 'noop' }
      }

      if (descriptor.status === 'stopped' || descriptor.status === 'broken') {
        return blockedWakeOutcome()
      }

      return { kind: 'noop' }
    },

    async status(handle) {
      const descriptor = await getRuntime(baseUrl, token, handle.id)
      if (!descriptor) {
        return { kind: 'stopped' }
      }
      return mapRuntimeStatus(descriptor.status)
    },

    async stop(handle) {
      await requestControlPlane<FirelineRuntimeDescriptor | null>(
        baseUrl,
        `/v1/runtimes/${encodeURIComponent(handle.id)}/stop`,
        {
          token,
          method: 'POST',
          allowNotFound: true,
        },
      )
    },
  }
}

function blockedWakeOutcome(): WakeOutcome {
  return {
    kind: 'blocked',
    reason: { kind: 'require_approval', scope: 'all' },
  }
}

function mapRuntimeStatus(status: FirelineRuntimeStatus): HostStatus {
  switch (status) {
    case 'ready':
    case 'busy':
      return { kind: 'running' }
    case 'idle':
      return { kind: 'idle' }
    case 'stopped':
      return { kind: 'stopped' }
    case 'broken':
      return { kind: 'error', message: 'Fireline runtime reported broken status' }
    case 'starting':
      return { kind: 'created' }
    case 'stale':
      return { kind: 'idle' }
  }
}

async function getRuntime(
  baseUrl: string,
  token: string | undefined,
  runtimeKey: string,
): Promise<FirelineRuntimeDescriptor | null> {
  return requestControlPlane<FirelineRuntimeDescriptor | null>(
    baseUrl,
    `/v1/runtimes/${encodeURIComponent(runtimeKey)}`,
    {
      token,
      allowNotFound: true,
    },
  )
}

async function waitForRuntimeReady(
  baseUrl: string,
  token: string | undefined,
  runtimeKey: string,
  timeoutMs: number,
): Promise<FirelineRuntimeDescriptor> {
  const deadline = Date.now() + timeoutMs

  while (Date.now() < deadline) {
    const descriptor = await getRuntime(baseUrl, token, runtimeKey)
    if (descriptor?.status === 'ready') {
      return descriptor
    }
    await delay(DEFAULT_POLL_INTERVAL_MS)
  }

  throw new Error(`timed out waiting for runtime '${runtimeKey}' to become ready`)
}

async function requestControlPlane<T>(
  baseUrl: string,
  path: string,
  options: {
    readonly token?: string
    readonly method?: string
    readonly body?: string
    readonly allowNotFound?: boolean
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
    const payload = (await response.json()) as { readonly error?: string }
    return payload.error ?? response.statusText
  } catch {
    return response.statusText
  }
}

function readMetadataString(
  metadata: ProvisionSpec['metadata'] | undefined,
  key: string,
): string | undefined {
  const value = metadata?.[key]
  return typeof value === 'string' ? value : undefined
}

function readMetadataNumber(
  metadata: ProvisionSpec['metadata'] | undefined,
  key: string,
): number | undefined {
  const value = metadata?.[key]
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}
