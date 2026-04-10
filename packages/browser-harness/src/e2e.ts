import { createBrowserFirelineClient } from '@fireline/client/browser'

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
    const client = createBrowserFirelineClient()
    await deleteRuntime().catch(() => undefined)

    const created = await fetchJson<{ runtime: HarnessRuntime }>(`${HARNESS_API_BASE}/runtime`, {
      method: 'POST',
      body: JSON.stringify({ agentId: 'fireline-testy-load' }),
    })

    const runtime = created.runtime
    const db = client.state.open({ stateStreamUrl: STATE_PROXY_URL })
    await db.preload()

    const acp = await client.acp.connect({ url: ACP_PROXY_URL })

    try {
      await acp.initialize({
        clientCapabilities: { fs: { readTextFile: false } },
        clientInfo: {
          name: '@fireline/browser-harness/e2e',
          version: '0.0.1',
          title: 'Fireline Browser E2E Driver',
        },
      })

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
      await client.close()
      await deleteRuntime().catch(() => undefined)
    }
  },
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
