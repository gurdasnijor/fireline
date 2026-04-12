// Fireline
import { createFirelineDB } from '@fireline/state'

// Third-party
import { eq } from '@tanstack/db'
import { useLiveQuery } from '@tanstack/react-db'
import { useEffect, useMemo, useState } from 'react'
import { useAcpClient } from 'use-acp'

export function TaskDashboard(props: {
  readonly acpUrl: string
  readonly stateStreamUrl: string
  readonly sessionId: string
}) {
  const db = useMemo(() => createFirelineDB({ stateStreamUrl: props.stateStreamUrl }), [props.stateStreamUrl])
  const [ready, setReady] = useState(false)
  const acp = useAcpClient({ wsUrl: props.acpUrl, autoConnect: true, initialSessionId: props.sessionId, sessionParams: { cwd: '/workspace', mcpServers: [] } })

  useEffect(() => {
    void db.preload().then(() => setReady(true))
    return () => db.close()
  }, [db])

  const turns = useLiveQuery((q) => q.from({ t: db.collections.promptTurns }).where(({ t }) => eq(t.sessionId, props.sessionId)), [db, props.sessionId])
  const chunks = useLiveQuery((q) => q.from({ c: db.collections.chunks }), [db])
  const permissions = useLiveQuery((q) => q.from({ p: db.collections.permissions }).where(({ p }) => eq(p.sessionId, props.sessionId)).where(({ p }) => eq(p.state, 'pending')), [db, props.sessionId])
  const optionId = acp.pendingPermission?.options[0]?.optionId

  if (!ready) return <section>Connecting to background task...</section>

  return (
    <section>
      <h2>Background Agent</h2>
      <p>ACP: {acp.connectionState.status}</p>
      <p>Turns: {turns.data?.length ?? 0}</p>
      <p>Pending approvals: {permissions.data?.length ?? 0}</p>
      {acp.pendingPermission && optionId ? <button onClick={() => acp.resolvePermission({ outcome: { outcome: 'selected', optionId } })}>Approve next tool call</button> : null}
      <pre>{chunks.data?.filter((entry) => turns.data?.some((turn) => turn.promptTurnId === entry.promptTurnId)).map((entry) => entry.content).join('') ?? 'Waiting for stream output...'}</pre>
    </section>
  )
}
