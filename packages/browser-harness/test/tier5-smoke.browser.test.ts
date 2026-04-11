import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { page, userEvent } from '@vitest/browser/context'
import { createElement } from 'react'
import { createRoot, type Root } from 'react-dom/client'

type MockRuntimeStatus = 'starting' | 'ready' | 'stopped'

type MockRuntime = {
  runtimeKey: string
  status: MockRuntimeStatus
  getCalls: number
  createBody: Record<string, unknown>
}

type MockRequestRecord = {
  readonly method: string
  readonly pathname: string
  readonly body?: unknown
}

type FetchMockState = {
  runtime: MockRuntime | null
  requests: MockRequestRecord[]
}

const launchableAgent = {
  id: 'fireline-testy-load',
  name: 'Fireline Testy Load',
  version: 'local',
  description: 'Mock launchable agent',
  launchable: true,
  distributionKind: 'command',
}

const resolvedAgentCommand = ['fireline-testy-load', '--mock']

let root: Root | null = null
let fetchState: FetchMockState

beforeEach(async () => {
  fetchState = {
    runtime: null,
    requests: [],
  }

  vi.stubGlobal('fetch', vi.fn((input: RequestInfo | URL, init?: RequestInit) =>
    handleFetch(input, init, fetchState),
  ) as typeof fetch)

  window.history.replaceState({}, '', '/')
  document.body.innerHTML = '<div id="root"></div>'

  const { App } = await import('../src/app.js')
  root = createRoot(document.getElementById('root')!)
  root.render(createElement(App))
})

afterEach(() => {
  root?.unmount()
  root = null
  document.body.innerHTML = ''
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('browser harness Tier 5 smoke flow', () => {
  it('launches, wakes, and stops a mocked Fireline runtime through the harness UI', async () => {
    const agentSelect = page.getByRole('combobox')
    await expect.element(agentSelect).toBeInTheDocument()

    await userEvent.selectOptions(await agentSelect.element(), 'fireline-testy-load')
    expect((await agentSelect.element())?.value).toBe('fireline-testy-load')

    await userEvent.click(page.getByRole('button', { name: 'Launch Agent' }))

    await expect.element(page.getByText(/^runtime-1$/)).toBeInTheDocument()
    await expect.element(page.getByText(/^running$/)).toBeInTheDocument()

    expect(fetchState.runtime?.createBody).toMatchObject({
      provider: 'local',
      host: '127.0.0.1',
      port: 0,
      name: 'browser-harness',
      agentCommand: resolvedAgentCommand,
      topology: { components: [] },
      resources: [],
      stateStream: 'fireline-harness-state',
    })

    await userEvent.click(page.getByRole('button', { name: 'Wake' }))

    await expect.element(page.getByText(/^wake$/)).toBeInTheDocument()
    await expect.element(page.getByText(/"kind": "noop"/)).toBeInTheDocument()

    await userEvent.click(page.getByRole('button', { name: 'Stop Runtime' }))

    await expect.element(page.getByText(/^runtime_stop$/)).toBeInTheDocument()

    const inspectorText = document.body.textContent ?? ''
    expect(inspectorText).toContain('sessionStatus')
    expect(inspectorText).toContain('handleId')
    expect((inspectorText.match(/not running/g) ?? []).length).toBeGreaterThanOrEqual(2)
    expect(inspectorText).not.toContain('runtime-1')

    expect(fetchState.requests).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ method: 'GET', pathname: '/api/agents' }),
        expect.objectContaining({ method: 'GET', pathname: '/api/resolve' }),
        expect.objectContaining({ method: 'POST', pathname: '/cp/v1/runtimes' }),
        expect.objectContaining({ method: 'GET', pathname: '/cp/v1/runtimes/runtime-1' }),
        expect.objectContaining({ method: 'POST', pathname: '/cp/v1/runtimes/runtime-1/stop' }),
      ]),
    )
  })
})

async function handleFetch(
  input: RequestInfo | URL,
  init: RequestInit | undefined,
  state: FetchMockState,
): Promise<Response> {
  const request = normalizeRequest(input, init)
  const url = new URL(request.url, window.location.origin)
  const method = request.method.toUpperCase()
  const pathname = url.pathname
  const body = await readJsonBody(request)

  state.requests.push({ method, pathname, body })

  if (method === 'GET' && pathname === '/api/agents') {
    return jsonResponse({ agents: [launchableAgent] })
  }

  if (method === 'GET' && pathname === '/api/resolve') {
    return jsonResponse({ agentCommand: resolvedAgentCommand })
  }

  if (method === 'POST' && pathname === '/cp/v1/runtimes') {
    const createBody = ensureObject(body)
    state.runtime = {
      runtimeKey: 'runtime-1',
      status: 'starting',
      getCalls: 0,
      createBody,
    }
    return jsonResponse({
      runtimeKey: state.runtime.runtimeKey,
      status: state.runtime.status,
    })
  }

  if (method === 'GET' && pathname === '/cp/v1/runtimes/runtime-1') {
    if (!state.runtime) {
      return jsonResponse({ error: 'not_found' }, 404)
    }

    state.runtime.getCalls += 1
    if (state.runtime.status === 'starting') {
      state.runtime.status = 'ready'
    }

    return jsonResponse({
      runtimeKey: state.runtime.runtimeKey,
      status: state.runtime.status,
    })
  }

  if (method === 'POST' && pathname === '/cp/v1/runtimes/runtime-1/stop') {
    if (state.runtime) {
      state.runtime.status = 'stopped'
    }

    return jsonResponse({
      runtimeKey: 'runtime-1',
      status: 'stopped',
    })
  }

  if (method === 'GET' && pathname === '/v1/stream/fireline-harness-state') {
    return jsonResponse([], 200, {
      'stream-up-to-date': 'true',
    })
  }

  throw new Error(`unexpected request: ${method} ${pathname}`)
}

function normalizeRequest(input: RequestInfo | URL, init?: RequestInit): Request {
  if (input instanceof Request) {
    if (!init) {
      return input
    }
    return new Request(input, init)
  }

  return new Request(input, init)
}

async function readJsonBody(request: Request): Promise<unknown> {
  const text = await request.clone().text()
  if (!text) {
    return undefined
  }
  return JSON.parse(text)
}

function ensureObject(value: unknown): Record<string, unknown> {
  if (typeof value === 'object' && value !== null && !Array.isArray(value)) {
    return value as Record<string, unknown>
  }
  throw new Error(`expected object body, got ${typeof value}`)
}

function jsonResponse(
  payload: unknown,
  status = 200,
  headers?: Record<string, string>,
): Response {
  return new Response(JSON.stringify(payload), {
    status,
    headers: {
      'content-type': 'application/json',
      ...(headers ?? {}),
    },
  })
}
