/// <reference types="vite/client" />

/**
 * Live deployment dashboard powered by Fireline durable state plus use-acp for
 * selected-session controls. No polling loop, no custom observer API.
 */

// Fireline
import { createFirelineDB } from '@fireline/state'

// Third-party
import { eq } from '@tanstack/db'
import { useLiveQuery } from '@tanstack/react-db'
import { useEffect, useMemo, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { useAcpClient } from 'use-acp'

// User code
const stateStreamUrl = import.meta.env.VITE_FIRELINE_STATE_STREAM_URL ?? 'http://127.0.0.1:7474/streams/state/demo'
const acpUrl = import.meta.env.VITE_FIRELINE_ACP_URL ?? 'ws://127.0.0.1:4440/v1/acp/demo'

function App() {
  const db = useMemo(() => createFirelineDB({ stateStreamUrl }), [])
  const [ready, setReady] = useState(false)
  useEffect(() => { void db.preload().then(() => setReady(true)); return () => db.close() }, [db])
  const sessions = useLiveQuery((q) => q.from({ s: db.collections.sessions }), [db])
  const turns = useLiveQuery((q) => q.from({ t: db.collections.promptTurns }), [db])
  const permissions = useLiveQuery((q) => q.from({ p: db.collections.permissions }).where(({ p }) => eq(p.state, 'pending')), [db])
  const edges = useLiveQuery((q) => q.from({ e: db.collections.childSessionEdges }), [db])
  const sessionId = sessions.data?.[0]?.sessionId ?? null
  const acp = useAcpClient({ wsUrl: acpUrl, autoConnect: true, initialSessionId: sessionId, sessionParams: { cwd: '/workspace', mcpServers: [] } })
  const optionId = acp.pendingPermission?.options[0]?.optionId
  if (!ready) return <main>Connecting to Fireline state...</main>
  return (
    <main style={{ fontFamily: 'ui-monospace, SFMono-Regular, monospace', padding: 24 }}>
      <h1>Fireline Live Dashboard</h1>
      <p>ACP {acp.connectionState.status} | Sessions {sessions.data?.length ?? 0} | Turns {turns.data?.length ?? 0} | Pending approvals {permissions.data?.length ?? 0}</p>
      <p>Child session edges {edges.data?.length ?? 0} | ACP notifications {acp.notifications.length}</p>
      {acp.pendingPermission && optionId ? <button onClick={() => acp.resolvePermission({ outcome: { outcome: 'selected', optionId } })}>Approve selected session</button> : null}
      <pre>{JSON.stringify({ sessions: sessions.data?.map((row) => row.sessionId), pendingApprovals: permissions.data?.map((row) => row.requestId), lineage: edges.data?.map((row) => `${row.parentSessionId} -> ${row.childSessionId}`) }, null, 2)}</pre>
    </main>
  )
}

createRoot(document.getElementById('app')!).render(<App />)
