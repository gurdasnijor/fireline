/**
 * Hosted-API Host satisfier — proves the `Host` primitive interface
 * accommodates a remote-hosted programming model alongside the
 * local-subprocess model implemented by `createFirelineHost`, per
 * `docs/proposals/client-primitives.md` and
 * `docs/proposals/runtime-host-split.md` §7.
 *
 * A Host provisions a runtime and returns a handle carrying the runtime's
 * ACP + state endpoints. Session lifecycle is an ACP data-plane concern,
 * not a Host-primitive verb — clients open an ACP connection against
 * `handle.acp.url` and call `session/new` directly.
 */
import type { SuspendReasonSpec } from '../core/index.js'
import type {
  Endpoint,
  Host,
  HostStatus,
  WakeOutcome,
} from '../host/index.js'

export interface HostedApiHostOptions {
  readonly endpointUrl: string
  readonly apiKey?: string
  readonly startupTimeoutMs?: number
  readonly pollIntervalMs?: number
}

type HostedApiRuntimeDescriptor = {
  readonly handle_id: string
  readonly status: string
  readonly acp?: Endpoint
  readonly state?: Endpoint
}

type HostedApiWakeResponse = {
  readonly outcome: 'noop' | 'advanced' | 'blocked'
  readonly steps?: number
  readonly reason?: SuspendReasonSpec
}

const DEFAULT_POLL_INTERVAL_MS = 100
const DEFAULT_STARTUP_TIMEOUT_MS = 20_000

export function createHostedApiHost(opts: HostedApiHostOptions): Host {
  const baseUrl = opts.endpointUrl.replace(/\/$/, '')
  const apiKey = opts.apiKey
  const pollIntervalMs = opts.pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS
  const startupTimeoutMs = opts.startupTimeoutMs ?? DEFAULT_STARTUP_TIMEOUT_MS

  return {
    async provision(spec) {
      const descriptor = await requestHostedApi<HostedApiRuntimeDescriptor>(baseUrl, '/v1/runtimes', {
        apiKey,
        method: 'POST',
        body: JSON.stringify(spec),
      })

      const ready = await waitForRuntimeReady(
        baseUrl,
        apiKey,
        descriptor.handle_id,
        startupTimeoutMs,
        pollIntervalMs,
      )

      return {
        id: ready.handle_id,
        kind: 'hosted-api',
        acp: ready.acp ?? { url: `${baseUrl}/v1/runtimes/${encodeURIComponent(ready.handle_id)}/acp` },
        state: ready.state ?? { url: `${baseUrl}/v1/runtimes/${encodeURIComponent(ready.handle_id)}/state` },
      }
    },

    async wake(handle) {
      const response = await requestHostedApi<HostedApiWakeResponse>(
        baseUrl,
        `/v1/runtimes/${encodeURIComponent(handle.id)}/wake`,
        {
          apiKey,
          method: 'POST',
        },
      )
      return mapWakeOutcome(response)
    },

    async status(handle) {
      const descriptor = await requestHostedApi<HostedApiRuntimeDescriptor | null>(
        baseUrl,
        `/v1/runtimes/${encodeURIComponent(handle.id)}`,
        {
          apiKey,
          allowNotFound: true,
        },
      )

      if (!descriptor) {
        return { kind: 'stopped' }
      }

      return mapHostedApiStatus(descriptor.status)
    },

    async stop(handle) {
      await requestHostedApi<HostedApiRuntimeDescriptor | null>(
        baseUrl,
        `/v1/runtimes/${encodeURIComponent(handle.id)}/stop`,
        {
          apiKey,
          method: 'POST',
          allowNotFound: true,
        },
      )
    },
  }
}

function mapWakeOutcome(response: HostedApiWakeResponse): WakeOutcome {
  switch (response.outcome) {
    case 'noop':
      return { kind: 'noop' }
    case 'advanced':
      return { kind: 'advanced', steps: response.steps ?? 1 }
    case 'blocked':
      return {
        kind: 'blocked',
        reason: response.reason ?? { kind: 'require_approval', scope: 'all' },
      }
  }
}

function mapHostedApiStatus(status: string): HostStatus {
  switch (status) {
    case 'ready':
    case 'busy':
      return { kind: 'running' }
    case 'idle':
      return { kind: 'idle' }
    case 'stopped':
      return { kind: 'stopped' }
    case 'starting':
      return { kind: 'created' }
    default:
      return { kind: 'error', message: status }
  }
}

async function getRuntime(
  baseUrl: string,
  apiKey: string | undefined,
  handleId: string,
): Promise<HostedApiRuntimeDescriptor | null> {
  return requestHostedApi<HostedApiRuntimeDescriptor | null>(
    baseUrl,
    `/v1/runtimes/${encodeURIComponent(handleId)}`,
    {
      apiKey,
      allowNotFound: true,
    },
  )
}

async function waitForRuntimeReady(
  baseUrl: string,
  apiKey: string | undefined,
  handleId: string,
  timeoutMs: number,
  pollIntervalMs: number,
): Promise<HostedApiRuntimeDescriptor> {
  const deadline = Date.now() + timeoutMs

  while (Date.now() < deadline) {
    const descriptor = await getRuntime(baseUrl, apiKey, handleId)
    if (!descriptor) {
      throw new Error(`hosted API runtime '${handleId}' was not found while waiting for readiness`)
    }

    if (descriptor.status === 'ready') {
      return descriptor
    }

    if (mapHostedApiStatus(descriptor.status).kind === 'error') {
      throw new Error(`hosted API runtime '${handleId}' reported error status '${descriptor.status}'`)
    }

    if (descriptor.status === 'stopped') {
      throw new Error(`hosted API runtime '${handleId}' stopped before becoming ready`)
    }

    await delay(pollIntervalMs)
  }

  throw new Error(`timed out waiting for hosted API runtime '${handleId}' to become ready`)
}

async function requestHostedApi<T>(
  baseUrl: string,
  path: string,
  options: {
    readonly apiKey?: string
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
      ...(options.apiKey ? { authorization: `Bearer ${options.apiKey}` } : {}),
    },
    body: options.body,
  })

  if (response.status === 404 && options.allowNotFound) {
    return null as T
  }

  if (!response.ok) {
    const message = await readHostedApiError(response)
    throw new Error(`${response.status} ${response.statusText}: ${message}`)
  }

  return (await response.json()) as T
}

async function readHostedApiError(response: Response): Promise<string> {
  try {
    const payload = (await response.json()) as { readonly error?: string }
    return payload.error ?? response.statusText
  } catch {
    return response.statusText
  }
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}
