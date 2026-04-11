/**
 * Hosted-API Host satisfier for the session lifecycle primitive described in
 * `docs/proposals/client-primitives.md`, aligned with
 * `docs/explorations/managed-agents-mapping.md`.
 */
import type { SessionSpec, SuspendReasonSpec } from '../core/index.js'
import type { Host, SessionStatus, WakeOutcome } from '../host/index.js'

export interface HostedApiHostOptions {
  readonly endpointUrl: string
  readonly apiKey?: string
  readonly startupTimeoutMs?: number
  readonly pollIntervalMs?: number
}

type HostedApiSessionDescriptor = {
  readonly handle_id: string
  readonly status: string
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
    async createSession(spec) {
      const descriptor = await requestHostedApi<HostedApiSessionDescriptor>(baseUrl, '/v1/sessions', {
        apiKey,
        method: 'POST',
        body: JSON.stringify(spec),
      })

      const ready = await waitForSessionReady(
        baseUrl,
        apiKey,
        descriptor.handle_id,
        startupTimeoutMs,
        pollIntervalMs,
      )

      return {
        id: ready.handle_id,
        kind: 'hosted-api',
      }
    },

    async wake(handle) {
      const response = await requestHostedApi<HostedApiWakeResponse>(
        baseUrl,
        `/v1/sessions/${encodeURIComponent(handle.id)}/wake`,
        {
          apiKey,
          method: 'POST',
        },
      )
      return mapWakeOutcome(response)
    },

    async status(handle) {
      const descriptor = await requestHostedApi<HostedApiSessionDescriptor | null>(
        baseUrl,
        `/v1/sessions/${encodeURIComponent(handle.id)}`,
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

    async stopSession(handle) {
      await requestHostedApi<HostedApiSessionDescriptor | null>(
        baseUrl,
        `/v1/sessions/${encodeURIComponent(handle.id)}/stop`,
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

function mapHostedApiStatus(status: string): SessionStatus {
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

async function getSession(
  baseUrl: string,
  apiKey: string | undefined,
  handleId: string,
): Promise<HostedApiSessionDescriptor | null> {
  return requestHostedApi<HostedApiSessionDescriptor | null>(
    baseUrl,
    `/v1/sessions/${encodeURIComponent(handleId)}`,
    {
      apiKey,
      allowNotFound: true,
    },
  )
}

async function waitForSessionReady(
  baseUrl: string,
  apiKey: string | undefined,
  handleId: string,
  timeoutMs: number,
  pollIntervalMs: number,
): Promise<HostedApiSessionDescriptor> {
  const deadline = Date.now() + timeoutMs

  while (Date.now() < deadline) {
    const descriptor = await getSession(baseUrl, apiKey, handleId)
    if (!descriptor) {
      throw new Error(`hosted API session '${handleId}' was not found while waiting for readiness`)
    }

    if (descriptor.status === 'ready') {
      return descriptor
    }

    if (mapHostedApiStatus(descriptor.status).kind === 'error') {
      throw new Error(`hosted API session '${handleId}' reported error status '${descriptor.status}'`)
    }

    if (descriptor.status === 'stopped') {
      throw new Error(`hosted API session '${handleId}' stopped before becoming ready`)
    }

    await delay(pollIntervalMs)
  }

  throw new Error(`timed out waiting for hosted API session '${handleId}' to become ready`)
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
