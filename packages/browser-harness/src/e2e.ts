import {
  ClientSideConnection,
  PROTOCOL_VERSION,
  type Client,
  type InitializeRequest,
  type RequestPermissionRequest,
  type RequestPermissionResponse,
  type Stream,
} from '@agentclientprotocol/sdk'
import { createFirelineDB } from '@fireline/state'

const STATE_STREAM_NAME =
  import.meta.env.VITE_FIRELINE_STATE_STREAM ?? 'fireline-harness-state'
const ACP_PROXY_URL = `ws://${window.location.host}/acp`
const STATE_PROXY_URL = `${window.location.origin}/v1/stream/${STATE_STREAM_NAME}`
const HARNESS_API_BASE = `${window.location.origin}/api`

type HarnessRuntime = {
  runtimeKey: string
  runtimeId: string
  status: string
  acpUrl: string
  stateStreamUrl: string
}

type BrowserE2EResult = {
  runtimeId: string
  runtimeStatus: string
  sessionId: string
  stopReason: string
  promptText: string
  promptTurnText: string | null
  chunkContent: string | null
  supportsLoadSession: boolean | null
}

declare global {
  interface Window {
    firelineE2E: {
      run(): Promise<BrowserE2EResult>
    }
  }
}

const root = document.getElementById('app')
if (!root) {
  throw new Error('missing e2e root')
}

root.textContent = 'Fireline browser e2e driver ready'

window.firelineE2E = {
  async run() {
    await deleteRuntime().catch(() => undefined)

    const created = await fetchJson<{ runtime: HarnessRuntime }>(`${HARNESS_API_BASE}/runtime`, {
      method: 'POST',
      body: JSON.stringify({ agentId: 'fireline-testy-load' }),
    })

    const runtime = created.runtime
    const db = createFirelineDB({ stateStreamUrl: STATE_PROXY_URL })
    await db.preload()

    const acp = await openBrowserAcpConnection({ url: ACP_PROXY_URL })

    try {
      await initializeAcp(acp.connection)

      const session = await acp.connection.newSession({
        cwd: '/',
        mcpServers: [],
      })

      const promptText = `hello from browser e2e ${crypto.randomUUID()}`
      const response = await acp.connection.prompt({
        sessionId: session.sessionId,
        prompt: [{ type: 'text', text: promptText }],
      })

      const sessionRow = await waitFor(
        () => db.collections.sessions.toArray.find((row) => row.sessionId === session.sessionId),
        5_000,
      )
      const promptTurn = await waitFor(
        () =>
          db.collections.promptTurns.toArray.find(
            (turn) =>
              turn.sessionId === session.sessionId &&
              turn.state === 'completed' &&
              turn.text === promptText,
          ),
        5_000,
      )
      const chunk = await waitFor(
        () =>
          db.collections.chunks.toArray.find(
            (entry) =>
              entry.promptTurnId === promptTurn?.promptTurnId && entry.content.includes('Hello'),
          ),
        5_000,
      )

      if (!sessionRow) {
        throw new Error('timed out waiting for session row in durable state')
      }
      if (!promptTurn) {
        throw new Error('timed out waiting for prompt turn in durable state')
      }
      if (!chunk) {
        throw new Error('timed out waiting for chunk row in durable state')
      }

      return {
        runtimeId: runtime.runtimeId,
        runtimeStatus: runtime.status,
        sessionId: session.sessionId,
        stopReason: response.stopReason,
        promptText,
        promptTurnText: promptTurn.text ?? null,
        chunkContent: chunk.content ?? null,
        supportsLoadSession: sessionRow.supportsLoadSession,
      }
    } finally {
      await acp.close()
      db.close()
      await deleteRuntime().catch(() => undefined)
    }
  },
}

async function initializeAcp(connection: ClientSideConnection): Promise<void> {
  const request: InitializeRequest = {
    protocolVersion: PROTOCOL_VERSION,
    clientCapabilities: { fs: { readTextFile: false } },
    clientInfo: {
      name: '@fireline/browser-harness/e2e',
      version: '0.0.1',
      title: 'Fireline Browser E2E Driver',
    },
  }

  await connection.initialize(request)
}

async function openBrowserAcpConnection(options: { url: string }): Promise<{
  connection: ClientSideConnection
  close(): Promise<void>
}> {
  const websocket = new WebSocket(options.url)
  await waitForSocketOpen(websocket)

  const connection = new ClientSideConnection(
    () => createClientHandler(),
    createWebSocketStream(websocket),
  )

  return {
    connection,
    async close() {
      if (websocket.readyState === WebSocket.OPEN || websocket.readyState === WebSocket.CONNECTING) {
        websocket.close()
      }
      await waitForSocketClose(websocket)
    },
  }
}

function createClientHandler(): Client {
  return {
    async requestPermission(_request: RequestPermissionRequest): Promise<RequestPermissionResponse> {
      return {
        outcome: {
          outcome: 'cancelled',
        },
      }
    },

    async sessionUpdate(): Promise<void> {
      // The e2e reads durable state directly and does not need live UI updates.
    },

    async writeTextFile(): Promise<never> {
      throw new Error('browser e2e does not implement writeTextFile')
    },
    async readTextFile(): Promise<never> {
      throw new Error('browser e2e does not implement readTextFile')
    },
    async createTerminal(): Promise<never> {
      throw new Error('browser e2e does not implement createTerminal')
    },
    async terminalOutput(): Promise<never> {
      throw new Error('browser e2e does not implement terminalOutput')
    },
    async releaseTerminal(): Promise<never> {
      throw new Error('browser e2e does not implement releaseTerminal')
    },
    async waitForTerminalExit(): Promise<never> {
      throw new Error('browser e2e does not implement waitForTerminalExit')
    },
    async killTerminal(): Promise<never> {
      throw new Error('browser e2e does not implement killTerminal')
    },
    async extMethod(method: string): Promise<Record<string, unknown>> {
      throw new Error(`browser e2e does not implement client ext method '${method}'`)
    },
    async extNotification(): Promise<void> {
      // Ignore unknown extension notifications.
    },
  }
}

function createWebSocketStream(ws: WebSocket): Stream {
  return {
    readable: new ReadableStream({
      start(controller) {
        ws.addEventListener('message', (event) => {
          toText(event.data)
            .then((text) => controller.enqueue(JSON.parse(text)))
            .catch((error) => controller.error(error))
        })
        ws.addEventListener('close', () => controller.close(), { once: true })
        ws.addEventListener('error', () => controller.error(new Error('WebSocket error')), {
          once: true,
        })
      },
    }),
    writable: new WritableStream({
      write(message) {
        ws.send(JSON.stringify(message))
      },
      close() {
        ws.close()
      },
      abort() {
        ws.close()
      },
    }),
  }
}

async function waitForSocketOpen(ws: WebSocket): Promise<void> {
  if (ws.readyState === WebSocket.OPEN) {
    return
  }

  await new Promise<void>((resolve, reject) => {
    const onOpen = () => {
      cleanup()
      resolve()
    }
    const onError = () => {
      cleanup()
      reject(new Error('WebSocket failed to open'))
    }
    const cleanup = () => {
      ws.removeEventListener('open', onOpen)
      ws.removeEventListener('error', onError)
    }
    ws.addEventListener('open', onOpen, { once: true })
    ws.addEventListener('error', onError, { once: true })
  })
}

async function waitForSocketClose(ws: WebSocket): Promise<void> {
  if (ws.readyState === WebSocket.CLOSED) {
    return
  }

  await new Promise<void>((resolve) => {
    ws.addEventListener('close', () => resolve(), { once: true })
  })
}

async function toText(data: Blob | ArrayBuffer | string): Promise<string> {
  if (typeof data === 'string') {
    return data
  }
  if (data instanceof Blob) {
    return await data.text()
  }
  return new TextDecoder().decode(data)
}

async function deleteRuntime(): Promise<void> {
  await fetchJson<{ runtime: null }>(`${HARNESS_API_BASE}/runtime`, {
    method: 'DELETE',
  })
}

async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, {
    ...init,
    headers: {
      'content-type': 'application/json',
      ...(init?.headers ?? {}),
    },
  })

  if (!response.ok) {
    throw new Error(`request failed (${response.status} ${response.statusText})`)
  }

  return (await response.json()) as T
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

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}
