// Fireline
import { createFirelineDB } from '@fireline/state'

// Third-party
import { eq } from '@tanstack/db'
import { useLiveQuery } from '@tanstack/react-db'
import { useEffect, useMemo, useState } from 'react'
import { useAcpClient } from 'use-acp'

export function ReviewDashboard(props: {
  readonly acpUrl: string
  readonly stateStreamUrl: string
  readonly sessionId: string
}) {
  const db = useMemo(() => createFirelineDB({ stateStreamUrl: props.stateStreamUrl }), [
    props.stateStreamUrl,
  ])
  const [ready, setReady] = useState(false)

  useEffect(() => {
    void db.preload().then(() => setReady(true))
    return () => db.close()
  }, [db])
  const acp = useAcpClient({
    wsUrl: props.acpUrl,
    autoConnect: true,
    initialSessionId: props.sessionId,
    sessionParams: { cwd: '/workspace', mcpServers: [] },
  })

  const turns = useLiveQuery(
    (q) =>
      q
        .from({ t: db.collections.promptTurns })
        .where(({ t }) => eq(t.sessionId, props.sessionId)),
    [db, props.sessionId],
  )
  const permissions = useLiveQuery(
    (q) =>
      q
        .from({ p: db.collections.permissions })
        .where(({ p }) => eq(p.sessionId, props.sessionId))
        .where(({ p }) => eq(p.state, 'pending')),
    [db, props.sessionId],
  )

  if (!ready) {
    return <section>Connecting to Fireline state...</section>
  }

  return (
    <section>
      <h2>Review Dashboard</h2>
      <p>ACP: {acp.connectionState.status}</p>
      <p>Turns: {turns.data?.length ?? 0}</p>
      <p>Pending approvals: {permissions.data?.length ?? 0}</p>
      <p>ACP notifications: {acp.notifications.length}</p>
      {acp.pendingPermission ? (
        <div>
          <p>{acp.pendingPermission.toolCall.title ?? 'Approval required'}</p>
          <button
            onClick={() =>
              acp.resolvePermission({
                outcome: {
                  outcome: 'selected',
                  optionId: acp.pendingPermission?.options[0]?.optionId ?? 'allow',
                },
              })
            }
          >
            Approve
          </button>
          <button onClick={() => acp.resolvePermission({ outcome: { outcome: 'cancelled' } })}>
            Deny
          </button>
        </div>
      ) : null}
      <pre>{turns.data?.map((turn) => turn.text ?? turn.state).join('\n') ?? 'No output yet'}</pre>
    </section>
  )
}
