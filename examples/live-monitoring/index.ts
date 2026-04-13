/// <reference types="vite/client" />
import fireline, { type FirelineDB } from '@fireline/client'
import { isToolCallSessionUpdate } from '@fireline/state'
import { useLiveQuery } from '@tanstack/react-db'
import { createElement as h, useEffect, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { useAcpClient } from 'use-acp'

const stateStreamUrl = import.meta.env.VITE_FIRELINE_STATE_STREAM_URL ?? 'http://127.0.0.1:7474/streams/state/demo'
const acpUrl = import.meta.env.VITE_FIRELINE_ACP_URL ?? 'ws://127.0.0.1:4440/v1/acp/demo'

function App() {
  const [db, setDb] = useState<FirelineDB | null>(null)

  useEffect(() => {
    let cancelled = false

    void fireline.db({ stateStreamUrl }).then((nextDb) => {
      if (cancelled) {
        nextDb.close()
        return
      }
      setDb(nextDb)
    })

    return () => {
      cancelled = true
      setDb((current) => {
        current?.close()
        return null
      })
    }
  }, [])

  return db
    ? h(MonitoringView, { db })
    : h('main', { style: { fontFamily: 'ui-monospace, monospace', padding: '24px' } }, 'Connecting to Fireline...')
}

function MonitoringView({ db }: { db: FirelineDB }) {
  const sessions = useLiveQuery((q) => q.from({ s: db.sessions }), [db])
  const turns = useLiveQuery((q) => q.from({ t: db.promptRequests }), [db])
  const approvals = useLiveQuery((q) => q.from({ p: db.permissions }), [db])
  const chunks = useLiveQuery((q) => q.from({ c: db.chunks }), [db])
  const acp = useAcpClient({ wsUrl: acpUrl, autoConnect: true, initialSessionId: sessions.data?.[0]?.sessionId ?? null, sessionParams: { cwd: '/workspace', mcpServers: [] } })
  const optionId = acp.pendingPermission?.options[0]?.optionId
  const pendingApprovals = approvals.data?.filter((row) => row.state === 'pending').length ?? 0
  const toolCalls = chunks.data?.filter((row) => isToolCallSessionUpdate(row.update)).length ?? 0

  return h('main', { style: { fontFamily: 'ui-monospace, monospace', padding: '24px' } }, [
    h('h1', { key: 'h' }, 'Fireline Live Monitoring'),
    h('pre', { key: 'p' }, JSON.stringify({ sessions: sessions.data?.length ?? 0, turns: turns.data?.length ?? 0, pendingApprovals, toolCalls, latestSession: sessions.data?.[0]?.sessionId ?? null }, null, 2)),
    optionId ? h('button', { key: 'b', onClick: () => acp.resolvePermission({ outcome: { outcome: 'selected', optionId } }) }, 'Approve next action') : null,
  ])
}

createRoot(document.getElementById('app')!).render(h(App))
