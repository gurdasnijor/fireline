import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import {
  ClientSideConnection,
  PROTOCOL_VERSION,
  type Client,
  type RequestPermissionRequest,
  type RequestPermissionResponse,
  type SessionNotification,
  type Stream,
} from '@agentclientprotocol/sdk'
import { useLiveQuery } from '@tanstack/react-db'
import { createFirelineDB, type FirelineDB } from '@fireline/state'
import { createFirelineHost } from '@fireline/client/host-fireline'
import type { Host, SessionHandle, SessionStatus } from '@fireline/client/host'

const STATE_STREAM_NAME =
  import.meta.env.VITE_FIRELINE_STATE_STREAM ?? 'fireline-harness-state'
const ACP_PROXY_URL = `ws://${window.location.host}/acp`
const STATE_PROXY_URL = `${window.location.origin}/v1/stream/${STATE_STREAM_NAME}`
const HARNESS_API_BASE = `${window.location.origin}/api`
const CONTROL_PLANE_URL = `${window.location.origin}/cp`

type HarnessStatus = 'disconnected' | 'connecting' | 'connected' | 'error'

type HarnessEvent = {
  type: string
  data: unknown
  timestamp: string
}

type PermissionResolver = (value: RequestPermissionResponse) => void

type CatalogAgent = {
  id: string
  name: string
  version: string
  description?: string
  launchable: boolean
  distributionKind?: string
  unavailableReason?: string
}

const DbContext = createContext<FirelineDB | null>(null)

function useDb(): FirelineDB {
  const db = useContext(DbContext)
  if (!db) {
    throw new Error('FirelineDB is not mounted')
  }
  return db
}

export function App() {
  const [dbEnabled, setDbEnabled] = useState(false)
  const [dbEpoch, setDbEpoch] = useState(0)
  const db = useMemo(
    () => (dbEnabled ? createFirelineDB({ stateStreamUrl: STATE_PROXY_URL }) : null),
    [dbEnabled, dbEpoch],
  )
  const [dbReady, setDbReady] = useState(false)
  const [dbError, setDbError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    setDbReady(false)
    setDbError(null)

    if (!dbEnabled || !db) {
      return
    }

    void db
      .preload()
      .then(() => {
        if (!cancelled) {
          setDbReady(true)
        }
      })
      .catch((error) => {
        if (!cancelled) {
          setDbError(toErrorMessage(error))
        }
      })

    return () => {
      cancelled = true
      db.close()
    }
  }, [db, dbEnabled])

  return (
    <DbContext.Provider value={db}>
      <div
        style={{
          display: 'flex',
          minHeight: '100vh',
          background: '#0b1020',
          color: '#d4d7dd',
          fontFamily:
            'ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
        }}
      >
        <div
          style={{
            flex: 1.2,
            minWidth: 0,
            display: 'flex',
            flexDirection: 'column',
            borderRight: '1px solid #1f2a44',
          }}
        >
          <HarnessHeader />
          <SessionHarness
            dbActive={dbEnabled}
            dbReady={dbReady}
            dbError={dbError}
            onHandleChanged={(_, status) => {
              setDbEnabled(status?.kind === 'running' || status?.kind === 'idle')
              setDbEpoch((current) => current + 1)
            }}
          />
        </div>
        <div
          style={{
            flex: 1,
            minWidth: 0,
            display: 'flex',
            flexDirection: 'column',
          }}
        >
          <StateExplorer active={dbEnabled} ready={dbReady} error={dbError} />
        </div>
      </div>
    </DbContext.Provider>
  )
}

function HarnessHeader() {
  return (
    <div
      style={{
        padding: '12px 16px',
        borderBottom: '1px solid #1f2a44',
        background:
          'linear-gradient(135deg, rgba(21,49,104,0.55), rgba(14,24,45,0.98))',
      }}
    >
      <div style={{ fontSize: 13, letterSpacing: 1.2, textTransform: 'uppercase', color: '#7ea6ff' }}>
        Fireline Browser Harness
      </div>
      <div style={{ marginTop: 6, fontSize: 12, color: '#95a2c0' }}>
        Live ACP over <code>/acp</code> plus durable state over{' '}
        <code>/v1/stream/{STATE_STREAM_NAME}</code>.
      </div>
    </div>
  )
}

function SessionHarness({
  dbActive,
  dbReady,
  dbError,
  onHandleChanged,
}: {
  dbActive: boolean
  dbReady: boolean
  dbError: string | null
  onHandleChanged(handle: SessionHandle | null, status: SessionStatus | null): void
}) {
  const host = useMemo<Host>(
    () =>
      createFirelineHost({
        controlPlaneUrl: CONTROL_PLANE_URL,
        sharedStateUrl: STATE_PROXY_URL,
      }),
    [],
  )
  const [status, setStatus] = useState<HarnessStatus>('disconnected')
  const [sessionId, setSessionId] = useState<string | null>(null)
  const [input, setInput] = useState('')
  const [events, setEvents] = useState<HarnessEvent[]>([])
  const [lastError, setLastError] = useState<string | null>(null)
  const [pendingPermission, setPendingPermission] = useState<RequestPermissionRequest | null>(null)
  const [agents, setAgents] = useState<CatalogAgent[]>([])
  const [selectedAgentId, setSelectedAgentId] = useState<string>('')
  const [handle, setHandle] = useState<SessionHandle | null>(null)
  const [sessionStatus, setSessionStatus] = useState<SessionStatus | null>(null)
  const [runtimePending, setRuntimePending] = useState(false)
  const connectionRef = useRef<ClientSideConnection | null>(null)
  const websocketRef = useRef<WebSocket | null>(null)
  const sessionIdRef = useRef<string | null>(null)
  const permissionResolverRef = useRef<PermissionResolver | null>(null)
  const bottomRef = useRef<HTMLDivElement | null>(null)
  const runtimeReady =
    sessionStatus?.kind === 'running' || sessionStatus?.kind === 'idle'

  useEffect(() => {
    void refreshAgents()

    return () => {
      void disconnect()
    }
  }, [])

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [events.length])

  async function openConnection(mode: 'new' | 'load') {
    if (!runtimeReady) {
      if (handle) {
        setLastError(
          `Runtime is not ready yet (${sessionStatus?.kind ?? 'unknown'})`,
        )
        setStatus('error')
        return
      }
      if (mode !== 'new') {
        setLastError('No active runtime to load a session from')
        setStatus('error')
        return
      }

      const launched = await launchRuntime()
      if (!launched) {
        return
      }
    }

    if (mode === 'load' && !sessionIdRef.current) {
      setLastError('No existing session id to load')
      setStatus('error')
      return
    }

    await disconnect({ preserveSessionId: true, clearError: true })
    setStatus('connecting')
    setLastError(null)
    pushEvent('connection', { mode, url: ACP_PROXY_URL })

    const websocket = new WebSocket(ACP_PROXY_URL)
    websocketRef.current = websocket

    try {
      await waitForSocketOpen(websocket)
      const connection = new ClientSideConnection(
        () =>
          createClientHandler({
            onPermission: (request, resolve) => {
              setPendingPermission(request)
              permissionResolverRef.current = resolve
            },
            onSessionUpdate: (notification) => {
              pushEvent('session_update', notification.update ?? notification)
            },
          }),
        createWebSocketStream(websocket),
      )
      connectionRef.current = connection

      await connection.initialize({
        protocolVersion: PROTOCOL_VERSION,
        clientCapabilities: { fs: { readTextFile: false } },
        clientInfo: {
          name: '@fireline/browser-harness',
          version: '0.0.1',
          title: 'Fireline Browser Harness',
        },
      })

      if (mode === 'new') {
        const response = await connection.newSession({
          cwd: '/',
          mcpServers: [],
        })
        sessionIdRef.current = response.sessionId
        setSessionId(response.sessionId)
        pushEvent('session_new', response)
      } else {
        const response = await connection.loadSession({
          sessionId: sessionIdRef.current!,
          cwd: '/',
          mcpServers: [],
        })
        pushEvent('session_load', response)
      }

      setStatus('connected')
    } catch (error) {
      const message = toErrorMessage(error)
      setLastError(message)
      setStatus('error')
      pushEvent('error', { message })
      await disconnect({ preserveSessionId: true, clearError: false })
    }
  }

  async function disconnect(options: { preserveSessionId?: boolean; clearError?: boolean } = {}) {
    const { preserveSessionId = true, clearError = true } = options
    const websocket = websocketRef.current
    websocketRef.current = null
    connectionRef.current = null
    permissionResolverRef.current = null
    setPendingPermission(null)

    if (!preserveSessionId) {
      sessionIdRef.current = null
      setSessionId(null)
    }
    if (clearError) {
      setLastError(null)
    }

    if (websocket && websocket.readyState !== WebSocket.CLOSED) {
      websocket.close()
      await waitForSocketClose(websocket)
    }

    setStatus('disconnected')
  }

  async function submitPrompt(event: React.FormEvent) {
    event.preventDefault()
    const connection = connectionRef.current
    const activeSessionId = sessionIdRef.current
    if (!connection || !activeSessionId || !input.trim()) {
      return
    }

    const text = input.trim()
    setInput('')
    pushEvent('user_prompt', { text })

    try {
      const response = await connection.prompt({
        sessionId: activeSessionId,
        prompt: [{ type: 'text', text }],
      })
      pushEvent('prompt_response', response)
    } catch (error) {
      const message = toErrorMessage(error)
      setLastError(message)
      pushEvent('error', { message })
    }
  }

  function resolvePermission(optionId?: string) {
    const resolver = permissionResolverRef.current
    if (!resolver) {
      return
    }

    if (optionId) {
      resolver({
        outcome: {
          outcome: 'selected',
          optionId,
        },
      })
    } else {
      resolver({
        outcome: {
          outcome: 'cancelled',
        },
      })
    }
    permissionResolverRef.current = null
    setPendingPermission(null)
  }

  function pushEvent(type: string, data: unknown) {
    setEvents((current) => [
      ...current,
      {
        type,
        data,
        timestamp: new Date().toISOString(),
      },
    ])
  }

  async function refreshAgents() {
    try {
      const response = await fetchJson<{ agents: CatalogAgent[] }>(`${HARNESS_API_BASE}/agents`)
      const launchable = response.agents.filter((agent) => agent.launchable)
      setAgents(launchable)
      setSelectedAgentId((current) => {
        if (current && launchable.some((agent) => agent.id === current)) {
          return current
        }
        return launchable[0]?.id ?? ''
      })
    } catch (error) {
      setLastError(toErrorMessage(error))
    }
  }

  async function refreshStatus(currentHandle: SessionHandle): Promise<SessionStatus | null> {
    try {
      const next = await host.status(currentHandle)
      setSessionStatus(next)
      onHandleChanged(currentHandle, next)
      return next
    } catch (error) {
      setLastError(toErrorMessage(error))
      return null
    }
  }

  async function launchRuntime(): Promise<boolean> {
    if (!selectedAgentId) {
      setLastError('No launchable agent selected')
      return false
    }

    setRuntimePending(true)
    setLastError(null)
    try {
      await disconnect({ preserveSessionId: false, clearError: true })
      setEvents([])

      const resolved = await fetchJson<{ agentCommand: readonly string[] }>(
        `${HARNESS_API_BASE}/resolve?agentId=${encodeURIComponent(selectedAgentId)}`,
      )

      const next = await host.createSession({
        agentCommand: resolved.agentCommand,
        metadata: {
          name: 'browser-harness',
          stateStream: STATE_STREAM_NAME,
          port: 4437,
        },
      })

      setHandle(next)
      const status = await refreshStatus(next)
      pushEvent('runtime_launch', {
        agentId: selectedAgentId,
        handle: next,
        status,
      })
      return true
    } catch (error) {
      const message = toErrorMessage(error)
      setLastError(message)
      pushEvent('error', { message })
      return false
    } finally {
      setRuntimePending(false)
    }
  }

  async function stopRuntime() {
    setRuntimePending(true)
    try {
      await disconnect({ preserveSessionId: false, clearError: true })
      setEvents([])
      const current = handle
      if (current) {
        await host.stopSession(current)
      }
      setHandle(null)
      setSessionStatus(null)
      onHandleChanged(null, null)
      pushEvent('runtime_stop', {})
    } catch (error) {
      const message = toErrorMessage(error)
      setLastError(message)
      pushEvent('error', { message })
    } finally {
      setRuntimePending(false)
    }
  }

  async function wakeSession() {
    if (!handle) {
      return
    }
    try {
      const outcome = await host.wake(handle)
      pushEvent('wake', outcome)
      await refreshStatus(handle)
    } catch (error) {
      const message = toErrorMessage(error)
      setLastError(message)
      pushEvent('error', { message })
    }
  }

  return (
    <>
      <div
        style={{
          display: 'flex',
          gap: 8,
          alignItems: 'center',
          padding: '10px 16px',
          borderBottom: '1px solid #1f2a44',
          background: '#0f1730',
          flexWrap: 'wrap',
        }}
      >
        <StatusPill status={status} />
        <code style={{ fontSize: 11, color: '#90a1c2' }}>
          {sessionId ? `session ${sessionId}` : 'no session'}
        </code>
        <select
          value={selectedAgentId}
          onChange={(event) => setSelectedAgentId(event.target.value)}
          disabled={runtimePending || agents.length === 0}
          style={{
            padding: '7px 10px',
            background: '#091121',
            color: '#d4d7dd',
            border: '1px solid #24324f',
            borderRadius: 6,
            fontSize: 12,
            minWidth: 220,
          }}
        >
          {agents.length === 0 ? (
            <option value="">No launchable agents</option>
          ) : (
            agents.map((agent) => (
              <option key={agent.id} value={agent.id}>
                {agent.name} ({agent.distributionKind ?? 'unknown'})
              </option>
            ))
          )}
        </select>
        <button
          style={buttonStyle('#1d4ed8')}
          disabled={!selectedAgentId || runtimePending}
          onClick={() => void launchRuntime()}
        >
          {handle ? 'Relaunch Agent' : 'Launch Agent'}
        </button>
        <button
          style={buttonStyle('#2563eb')}
          disabled={runtimePending || Boolean(handle && !runtimeReady)}
          onClick={() => void openConnection('new')}
        >
          New Session
        </button>
        <button
          style={buttonStyle('#0f766e')}
          disabled={!sessionId || status === 'connecting' || !runtimeReady}
          onClick={() => void openConnection('load')}
        >
          Reconnect + Load
        </button>
        <button
          style={buttonStyle('#475569')}
          disabled={status === 'disconnected'}
          onClick={() => void disconnect({ preserveSessionId: true })}
        >
          Disconnect
        </button>
        <button
          style={buttonStyle('#0e7490')}
          disabled={!handle || runtimePending}
          onClick={() => void wakeSession()}
        >
          Wake
        </button>
        <button
          style={buttonStyle('#6b7280')}
          disabled={!handle || runtimePending}
          onClick={() => void stopRuntime()}
        >
          Stop Runtime
        </button>
        <button
          style={buttonStyle('#7c2d12')}
          onClick={() => {
            void disconnect({ preserveSessionId: false })
            setEvents([])
          }}
        >
          Reset
        </button>
      </div>

      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'minmax(0, 1fr) minmax(280px, 340px)',
          flex: 1,
          minHeight: 0,
        }}
      >
        <div style={{ display: 'flex', flexDirection: 'column', minWidth: 0 }}>
          <div style={{ flex: 1, overflow: 'auto', padding: 14 }}>
            {events.map((event, index) => (
              <EventRow key={`${event.timestamp}-${index}`} event={event} />
            ))}
            <div ref={bottomRef} />
          </div>

          {pendingPermission && (
            <div
              style={{
                padding: '10px 14px',
                background: '#151b33',
                borderTop: '1px solid #1f2a44',
              }}
            >
              <div style={{ marginBottom: 8, fontSize: 12, color: '#fbbf24' }}>
                Permission request
              </div>
              <div style={{ fontSize: 11, color: '#b8c2d8', marginBottom: 10 }}>
                {pendingPermission.toolCall?.title ?? 'Agent requested permission'}
              </div>
              <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                {pendingPermission.options?.map((option) => (
                  <button
                    key={option.optionId}
                    style={buttonStyle('#166534')}
                    onClick={() => resolvePermission(option.optionId)}
                  >
                    {option.name}
                  </button>
                ))}
                <button style={buttonStyle('#475569')} onClick={() => resolvePermission()}>
                  Cancel
                </button>
              </div>
            </div>
          )}

          <form
            onSubmit={submitPrompt}
            style={{
              display: 'flex',
              gap: 8,
              padding: '12px 16px',
              borderTop: '1px solid #1f2a44',
              background: '#0f1730',
            }}
          >
            <input
              value={input}
              onChange={(event) => setInput(event.target.value)}
              placeholder={
                status === 'connected'
                  ? 'Enter a prompt. Plain text will return Hello, world! against fireline-testy-load.'
                  : 'Click New Session to begin'
              }
              disabled={status !== 'connected'}
              style={{
                flex: 1,
                minWidth: 0,
                padding: '10px 12px',
                background: '#091121',
                color: '#d4d7dd',
                border: '1px solid #24324f',
                borderRadius: 6,
                fontSize: 13,
              }}
            />
            <button
              type="submit"
              disabled={status !== 'connected' || !input.trim()}
              style={buttonStyle('#2563eb')}
            >
              Send
            </button>
          </form>
        </div>

        <div
          style={{
            borderLeft: '1px solid #1f2a44',
            background: '#0f1730',
            overflow: 'auto',
            minWidth: 0,
          }}
        >
          <InspectorCard title="Current Session">
            <KeyValueRow label="status" value={status} />
            <KeyValueRow label="sessionId" value={sessionId ?? 'none'} mono />
            <KeyValueRow
              label="sessionStatus"
              value={sessionStatus?.kind ?? 'not running'}
            />
            <KeyValueRow
              label="lastError"
              value={lastError ?? 'none'}
            />
            <KeyValueRow
              label="handleId"
              value={handle?.id ?? 'not running'}
              mono
            />
            {dbActive ? (
              <CurrentSessionFields sessionId={sessionId} ready={dbReady} error={dbError} />
            ) : (
              <KeyValueRow label="statePlane" value="idle until runtime is ready" />
            )}
          </InspectorCard>

          <InspectorCard title="Recent Turns">
            {!dbActive ? (
              <EmptyState label="Launch a ready runtime to observe durable state" />
            ) : dbError ? (
              <EmptyState label={`State stream error: ${dbError}`} />
            ) : !dbReady ? (
              <EmptyState label="Connecting durable state…" />
            ) : (
              <RecentTurnsPanel sessionId={sessionId} />
            )}
          </InspectorCard>
        </div>
      </div>
    </>
  )
}

function StateExplorer({
  active,
  ready,
  error,
}: {
  active: boolean
  ready: boolean
  error: string | null
}) {
  const [tab, setTab] = useState<'sessions' | 'turns' | 'edges' | 'chunks' | 'connections'>(
    'sessions',
  )

  return (
    <>
      <div
        style={{
          display: 'flex',
          gap: 6,
          alignItems: 'center',
          padding: '10px 14px',
          borderBottom: '1px solid #1f2a44',
          background: '#0f1730',
          flexWrap: 'wrap',
        }}
      >
        <div style={{ fontSize: 12, fontWeight: 600, color: '#9aa9c7', marginRight: 'auto' }}>
          Durable State
        </div>
        {(['sessions', 'turns', 'edges', 'chunks', 'connections'] as const).map((name) => (
          <button
            key={name}
            onClick={() => setTab(name)}
            style={{
              ...buttonStyle(tab === name ? '#2563eb' : '#334155'),
              padding: '4px 9px',
              fontSize: 11,
              textTransform: 'capitalize',
            }}
          >
            {name}
          </button>
        ))}
      </div>

      <div style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
        {!active ? (
          <EmptyState label="Idle until a runtime is ready" />
        ) : error ? (
          <EmptyState label={`State stream error: ${error}`} />
        ) : !ready ? (
          <EmptyState label="Connecting durable state…" />
        ) : (
          <>
            {tab === 'sessions' && <SessionsView />}
            {tab === 'turns' && <TurnsView />}
            {tab === 'edges' && <EdgesView />}
            {tab === 'chunks' && <ChunksView />}
            {tab === 'connections' && <ConnectionsView />}
          </>
        )}
      </div>
    </>
  )
}

function CurrentSessionFields({
  sessionId,
  ready,
  error,
}: {
  sessionId: string | null
  ready: boolean
  error: string | null
}) {
  const db = useDb()
  const query = useLiveQuery((q) => q.from({ s: db.collections.sessions }))
  const currentSessionRow = useMemo(() => {
    if (!sessionId) {
      return null
    }
    return query.data?.find((row) => row.sessionId === sessionId) ?? null
  }, [query.data, sessionId])

  if (error) {
    return <KeyValueRow label="statePlane" value={error} />
  }
  if (!ready) {
    return <KeyValueRow label="statePlane" value="connecting" />
  }

  return (
    <>
      <KeyValueRow
        label="supportsLoadSession"
        value={currentSessionRow ? String(currentSessionRow.supportsLoadSession) : 'n/a'}
      />
      <KeyValueRow label="traceId" value={currentSessionRow?.traceId ?? 'n/a'} mono />
      <KeyValueRow label="sessionState" value={currentSessionRow?.state ?? 'n/a'} />
    </>
  )
}

function RecentTurnsPanel({ sessionId }: { sessionId: string | null }) {
  const db = useDb()
  const query = useLiveQuery((q) =>
    q.from({ t: db.collections.promptTurns }).orderBy(({ t }) => t.startedAt, 'desc'),
  )
  const currentTurns = useMemo(() => {
    if (!sessionId) {
      return []
    }
    return (query.data ?? []).filter((turn) => turn.sessionId === sessionId).slice(0, 5)
  }, [query.data, sessionId])

  if (currentTurns.length === 0) {
    return <EmptyState label="No turns for current session yet" />
  }

  return (
    <>
      {currentTurns.map((turn) => (
        <div
          key={turn.promptTurnId}
          style={{
            padding: '8px 0',
            borderBottom: '1px solid #1f2a44',
          }}
        >
          <div style={{ color: stateColor(turn.state), fontSize: 11 }}>{turn.state}</div>
          <div style={{ color: '#d4d7dd', fontSize: 12, marginTop: 2 }}>
            {turn.text ?? '(no text)'}
          </div>
          <div style={{ color: '#7081a3', fontSize: 10, marginTop: 3 }}>
            {new Date(turn.startedAt).toLocaleTimeString()}
          </div>
        </div>
      ))}
    </>
  )
}

function SessionsView() {
  const db = useDb()
  const query = useLiveQuery((q) =>
    q.from({ s: db.collections.sessions }).orderBy(({ s }) => s.createdAt, 'desc'),
  )

  return (
    <ListView loading={query.isLoading} empty="No sessions yet">
      {(query.data ?? []).map((session) => (
        <ListItem key={session.sessionId}>
          <div style={{ color: stateColor(session.state), fontSize: 11 }}>{session.state}</div>
          <div style={{ color: '#d4d7dd', marginTop: 2 }}>{session.runtimeId}</div>
          <div style={{ color: '#8fa1c6', fontSize: 10, marginTop: 4, fontFamily: 'ui-monospace, monospace' }}>
            {session.sessionId}
          </div>
        </ListItem>
      ))}
    </ListView>
  )
}

function TurnsView() {
  const db = useDb()
  const query = useLiveQuery((q) =>
    q.from({ t: db.collections.promptTurns }).orderBy(({ t }) => t.startedAt, 'desc'),
  )

  return (
    <ListView loading={query.isLoading} empty="No prompt turns yet">
      {(query.data ?? []).map((turn) => (
        <ListItem key={turn.promptTurnId}>
          <div style={{ color: stateColor(turn.state), fontSize: 11 }}>
            {turn.state}
            {turn.stopReason ? ` · ${turn.stopReason}` : ''}
          </div>
          <div style={{ color: '#d4d7dd', marginTop: 2 }}>{turn.text ?? '(no text)'}</div>
          <div style={{ color: '#8fa1c6', fontSize: 10, marginTop: 4 }}>
            session {truncate(turn.sessionId)} · trace {truncate(turn.traceId ?? 'n/a')}
          </div>
        </ListItem>
      ))}
    </ListView>
  )
}

function EdgesView() {
  const db = useDb()
  const query = useLiveQuery((q) =>
    q.from({ e: db.collections.childSessionEdges }).orderBy(({ e }) => e.createdAt, 'desc'),
  )

  return (
    <ListView loading={query.isLoading} empty="No child-session edges yet">
      {(query.data ?? []).map((edge) => (
        <ListItem key={edge.edgeId}>
          <div style={{ color: '#7ea6ff', fontSize: 11 }}>
            {truncate(edge.parentRuntimeId)} → {truncate(edge.childRuntimeId)}
          </div>
          <div style={{ color: '#d4d7dd', marginTop: 2 }}>
            parent turn {truncate(edge.parentPromptTurnId)}
          </div>
          <div style={{ color: '#8fa1c6', fontSize: 10, marginTop: 4 }}>
            child session {truncate(edge.childSessionId)}
          </div>
        </ListItem>
      ))}
    </ListView>
  )
}

function ChunksView() {
  const db = useDb()
  const query = useLiveQuery((q) =>
    q.from({ c: db.collections.chunks }).orderBy(({ c }) => c.createdAt, 'desc'),
  )

  return (
    <ListView loading={query.isLoading} empty="No chunks yet">
      {(query.data ?? []).slice(0, 100).map((chunk) => (
        <ListItem key={chunk.chunkId}>
          <div style={{ color: chunkTypeColor(chunk.type), fontSize: 11 }}>
            {chunk.type} · seq {chunk.seq}
          </div>
          <div style={{ color: '#d4d7dd', marginTop: 2, whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>
            {chunk.content}
          </div>
          <div style={{ color: '#8fa1c6', fontSize: 10, marginTop: 4 }}>
            turn {truncate(chunk.promptTurnId)}
          </div>
        </ListItem>
      ))}
    </ListView>
  )
}

function ConnectionsView() {
  const db = useDb()
  const query = useLiveQuery((q) =>
    q.from({ c: db.collections.connections }).orderBy(({ c }) => c.updatedAt, 'desc'),
  )

  return (
    <ListView loading={query.isLoading} empty="No connections yet">
      {(query.data ?? []).map((connection) => (
        <ListItem key={connection.logicalConnectionId}>
          <div style={{ color: stateColor(connection.state), fontSize: 11 }}>{connection.state}</div>
          <div style={{ color: '#d4d7dd', marginTop: 2 }}>
            {truncate(connection.logicalConnectionId)}
          </div>
          <div style={{ color: '#8fa1c6', fontSize: 10, marginTop: 4 }}>
            latest session {truncate(connection.latestSessionId ?? 'n/a')}
          </div>
        </ListItem>
      ))}
    </ListView>
  )
}

function createClientHandler(options: {
  onPermission(request: RequestPermissionRequest, resolve: PermissionResolver): void
  onSessionUpdate(notification: SessionNotification): void
}): Client {
  return {
    async requestPermission(request: RequestPermissionRequest): Promise<RequestPermissionResponse> {
      return await new Promise((resolve) => {
        options.onPermission(request, resolve)
      })
    },

    async sessionUpdate(notification: SessionNotification): Promise<void> {
      options.onSessionUpdate(notification)
    },

    async writeTextFile(): Promise<never> {
      throw new Error('Browser harness does not implement writeTextFile')
    },
    async readTextFile(): Promise<never> {
      throw new Error('Browser harness does not implement readTextFile')
    },
    async createTerminal(): Promise<never> {
      throw new Error('Browser harness does not implement createTerminal')
    },
    async terminalOutput(): Promise<never> {
      throw new Error('Browser harness does not implement terminalOutput')
    },
    async releaseTerminal(): Promise<never> {
      throw new Error('Browser harness does not implement releaseTerminal')
    },
    async waitForTerminalExit(): Promise<never> {
      throw new Error('Browser harness does not implement waitForTerminalExit')
    },
    async killTerminal(): Promise<never> {
      throw new Error('Browser harness does not implement killTerminal')
    },
    async extMethod(method: string): Promise<Record<string, unknown>> {
      throw new Error(`Browser harness does not implement client ext method '${method}'`)
    },
    async extNotification(): Promise<void> {
      // Ignore unknown extension notifications in the harness.
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

function buttonStyle(background: string) {
  return {
    padding: '7px 11px',
    background,
    color: '#f8fafc',
    border: 'none',
    borderRadius: 6,
    cursor: 'pointer',
    fontSize: 12,
  }
}

function StatusPill({ status }: { status: HarnessStatus }) {
  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        padding: '4px 10px',
        borderRadius: 999,
        background: '#0b1020',
        fontSize: 11,
        color: statusColor(status),
        border: `1px solid ${statusColor(status)}`,
      }}
    >
      ● {status}
    </span>
  )
}

function EventRow({ event }: { event: HarnessEvent }) {
  if (event.type === 'user_prompt') {
    return (
      <div style={{ padding: '5px 0', color: '#7ea6ff', fontSize: 13 }}>
        {'> '} {(event.data as { text: string }).text}
      </div>
    )
  }

  if (event.type === 'session_update') {
    const update = event.data as Record<string, unknown>
    const kind = String(update.sessionUpdate ?? update.type ?? '')

    if (kind === 'agent_message_chunk' || kind === 'agentMessageChunk') {
      const text =
        String(
          (update.content as Record<string, unknown> | undefined)?.text ??
            ((update.content as Record<string, unknown> | undefined)?.content as
              | Record<string, unknown>
              | undefined)?.text ??
            '',
        ) || ''

      return (
        <div style={{ padding: '3px 0', color: '#d4d7dd', whiteSpace: 'pre-wrap' }}>{text}</div>
      )
    }

    return (
      <div style={{ padding: '3px 0', color: '#93a2c8', fontSize: 11 }}>
        [{kind || 'session_update'}]
      </div>
    )
  }

  return (
    <div style={{ padding: '4px 0' }}>
      <div style={{ color: '#93a2c8', fontSize: 10 }}>{event.type}</div>
      <pre
        style={{
          margin: '4px 0 0',
          padding: '8px 10px',
          background: '#091121',
          color: '#c0c9dc',
          borderRadius: 6,
          fontSize: 11,
          overflowX: 'auto',
          whiteSpace: 'pre-wrap',
          wordBreak: 'break-word',
        }}
      >
        {JSON.stringify(event.data, null, 2)}
      </pre>
    </div>
  )
}

function InspectorCard({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div style={{ padding: '14px 16px', borderBottom: '1px solid #1f2a44' }}>
      <div style={{ fontSize: 12, fontWeight: 600, color: '#9aa9c7', marginBottom: 10 }}>
        {title}
      </div>
      {children}
    </div>
  )
}

function KeyValueRow({
  label,
  value,
  mono = false,
}: {
  label: string
  value: string
  mono?: boolean
}) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '120px minmax(0, 1fr)', gap: 10, marginBottom: 6 }}>
      <div style={{ color: '#7081a3', fontSize: 11 }}>{label}</div>
      <div
        style={{
          color: '#d4d7dd',
          fontSize: 11,
          fontFamily: mono ? 'ui-monospace, monospace' : undefined,
          wordBreak: 'break-word',
        }}
      >
        {value}
      </div>
    </div>
  )
}

function ListView({
  loading,
  empty,
  children,
}: {
  loading: boolean
  empty: string
  children: ReactNode
}) {
  const hasItems = Array.isArray(children) ? children.length > 0 : Boolean(children)

  if (loading && !hasItems) {
    return <EmptyState label="Loading…" />
  }

  if (!hasItems) {
    return <EmptyState label={empty} />
  }

  return <div style={{ padding: '10px 12px' }}>{children}</div>
}

function ListItem({ children }: { children: ReactNode }) {
  return (
    <div
      style={{
        padding: '8px 10px',
        marginBottom: 6,
        background: '#0b1020',
        border: '1px solid #1f2a44',
        borderRadius: 6,
        fontSize: 12,
      }}
    >
      {children}
    </div>
  )
}

function EmptyState({ label }: { label: string }) {
  return <div style={{ padding: 14, color: '#7081a3', fontSize: 12 }}>{label}</div>
}

function truncate(value: string, width = 12): string {
  if (value.length <= width) {
    return value
  }
  return `${value.slice(0, width)}…`
}

function statusColor(status: HarnessStatus): string {
  switch (status) {
    case 'connected':
      return '#4ade80'
    case 'connecting':
      return '#fbbf24'
    case 'error':
      return '#ef4444'
    default:
      return '#94a3b8'
  }
}

function stateColor(state: string): string {
  switch (state) {
    case 'attached':
    case 'active':
    case 'open':
    case 'ready':
      return '#4ade80'
    case 'completed':
    case 'resolved':
    case 'stopped':
      return '#60a5fa'
    case 'queued':
    case 'pending':
    case 'created':
    case 'starting':
      return '#fbbf24'
    case 'cancelled':
    case 'broken':
    case 'closed':
    case 'timed_out':
      return '#ef4444'
    default:
      return '#94a3b8'
  }
}

function chunkTypeColor(type: string): string {
  switch (type) {
    case 'text':
      return '#7ea6ff'
    case 'tool_call':
      return '#f59e0b'
    case 'tool_result':
      return '#22c55e'
    case 'error':
      return '#ef4444'
    default:
      return '#94a3b8'
  }
}

function toErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message
  }
  return String(error)
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
    let message = `${response.status} ${response.statusText}`
    try {
      const payload = (await response.json()) as { error?: string }
      if (payload?.error) {
        message = payload.error
      }
    } catch {
      // Ignore malformed JSON errors and fall back to the HTTP status.
    }
    throw new Error(message)
  }

  return (await response.json()) as T
}
