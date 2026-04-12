/// <reference types="vite/client" />
import { createFirelineDB } from '@fireline/state'
import { eq } from '@tanstack/db'
import { useLiveQuery } from '@tanstack/react-db'
import { createElement as h, useEffect, useMemo, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { useAcpClient } from 'use-acp'

const stateStreamUrl = import.meta.env.VITE_FIRELINE_STATE_STREAM_URL ?? 'http://127.0.0.1:7474/streams/state/demo'
const acpUrl = import.meta.env.VITE_FIRELINE_ACP_URL ?? 'ws://127.0.0.1:4440/v1/acp/demo'
function App() {
  const db = useMemo(() => createFirelineDB({ stateStreamUrl }), []), [ready, setReady] = useState(false)
  useEffect(() => { void db.preload().then(() => setReady(true)); return () => db.close() }, [db])
  const sessions = useLiveQuery((q) => q.from({ s: db.collections.sessions }), [db])
  const turns = useLiveQuery((q) => q.from({ t: db.collections.promptTurns }), [db])
  const approvals = useLiveQuery((q) => q.from({ p: db.collections.permissions }).where(({ p }) => eq(p.state, 'pending')), [db])
  const toolCalls = useLiveQuery((q) => q.from({ c: db.collections.chunks }).where(({ c }) => eq(c.type, 'tool_call')), [db])
  const acp = useAcpClient({ wsUrl: acpUrl, autoConnect: true, initialSessionId: sessions.data?.[0]?.sessionId ?? null, sessionParams: { cwd: '/workspace', mcpServers: [] } })
  const optionId = acp.pendingPermission?.options[0]?.optionId
  return h('main', { style: { fontFamily: 'ui-monospace, monospace', padding: '24px' } }, ready ? [h('h1', { key: 'h' }, 'Fireline Live Monitoring'), h('pre', { key: 'p' }, JSON.stringify({ sessions: sessions.data?.length ?? 0, turns: turns.data?.length ?? 0, pendingApprovals: approvals.data?.length ?? 0, toolCalls: toolCalls.data?.length ?? 0, latestSession: sessions.data?.[0]?.sessionId ?? null }, null, 2)), optionId ? h('button', { key: 'b', onClick: () => acp.resolvePermission({ outcome: { outcome: 'selected', optionId } }) }, 'Approve next action') : null] : 'Connecting to Fireline...')
}
createRoot(document.getElementById('app')!).render(h(App))
