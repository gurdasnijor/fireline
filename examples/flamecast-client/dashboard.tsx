// Fireline
import { createFirelineDB } from '@fireline/state'

// Third-party
import { eq } from '@tanstack/db'
import { useLiveQuery } from '@tanstack/react-db'
import { useEffect, useMemo, useState } from 'react'

export function ReviewDashboard(props: {
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
      <p>Turns: {turns.data?.length ?? 0}</p>
      <p>Pending approvals: {permissions.data?.length ?? 0}</p>
      <pre>{turns.data?.map((turn) => turn.text ?? turn.state).join('\n') ?? 'No output yet'}</pre>
    </section>
  )
}
