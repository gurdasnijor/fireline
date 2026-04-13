/// <reference types="vite/client" />
import fireline, { type FirelineDB } from '@fireline/client'
import {
  extractChunkTextPreview,
  isToolCallSessionUpdate,
  type ChunkRow,
  type PermissionRow,
  type PromptRequestRow,
  type SessionRow,
} from '@fireline/state'
import { createElement as h, useEffect, useState, type CSSProperties } from 'react'
import { createRoot } from 'react-dom/client'

const stateStreamUrl =
  import.meta.env.VITE_FIRELINE_STATE_STREAM_URL ??
  'http://127.0.0.1:7474/streams/state/demo'

type MonitorRows = {
  sessions: SessionRow[]
  promptRequests: PromptRequestRow[]
  permissions: PermissionRow[]
  chunks: ChunkRow[]
}

type SessionCard = {
  sessionId: string
  state: SessionRow['state']
  requestCount: number
  activeRequests: number
  pendingApprovals: number
  latestActivity: string
  updatedAt: number
}

type MonitorSnapshot = {
  sessions: number
  activeSessions: number
  activeRequests: number
  queuedRequests: number
  completedRequests: number
  pendingApprovals: number
  toolCalls: number
  latestActivity: string
  cards: SessionCard[]
}

const emptyRows = (): MonitorRows => ({
  sessions: [],
  promptRequests: [],
  permissions: [],
  chunks: [],
})

const emptySnapshot: MonitorSnapshot = {
  sessions: 0,
  activeSessions: 0,
  activeRequests: 0,
  queuedRequests: 0,
  completedRequests: 0,
  pendingApprovals: 0,
  toolCalls: 0,
  latestActivity: 'Waiting for the first durable event.',
  cards: [],
}

const pageStyle: CSSProperties = {
  minHeight: '100vh',
  margin: 0,
  background:
    'radial-gradient(circle at top left, rgba(255, 215, 140, 0.35), transparent 28%), linear-gradient(180deg, #11131a 0%, #191d28 58%, #0d1017 100%)',
  color: '#f7f0df',
  fontFamily: '"Avenir Next", "Segoe UI", sans-serif',
}

const shellStyle: CSSProperties = {
  maxWidth: '1120px',
  margin: '0 auto',
  padding: '48px 24px 64px',
}

const gridStyle: CSSProperties = {
  display: 'grid',
  gap: '16px',
  gridTemplateColumns: 'repeat(auto-fit, minmax(180px, 1fr))',
}

const cardStyle: CSSProperties = {
  background: 'rgba(16, 21, 31, 0.84)',
  border: '1px solid rgba(247, 240, 223, 0.12)',
  borderRadius: '20px',
  padding: '18px 20px',
  boxShadow: '0 22px 60px rgba(0, 0, 0, 0.24)',
}

const labelStyle: CSSProperties = {
  display: 'block',
  fontSize: '12px',
  letterSpacing: '0.12em',
  textTransform: 'uppercase',
  color: '#c7bda7',
  marginBottom: '10px',
}

const valueStyle: CSSProperties = {
  display: 'block',
  fontSize: '30px',
  fontWeight: 700,
  lineHeight: '1.1',
}

function App() {
  const [status, setStatus] = useState<'connecting' | 'connected' | 'error'>(
    'connecting',
  )
  const [snapshot, setSnapshot] = useState<MonitorSnapshot>(emptySnapshot)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    let animationFrame = 0
    let db: FirelineDB | null = null
    const rows = emptyRows()
    const subscriptions: Array<{ unsubscribe(): void }> = []

    // Chunk-heavy sessions can update quickly; batch subscription churn into one paint.
    const publish = () => {
      if (animationFrame !== 0) {
        return
      }

      animationFrame = window.requestAnimationFrame(() => {
        animationFrame = 0
        if (cancelled) {
          return
        }
        setSnapshot(buildSnapshot(rows))
        setStatus('connected')
      })
    }

    void fireline
      .db({ stateStreamUrl })
      .then((nextDb) => {
        if (cancelled) {
          nextDb.close()
          return
        }

        db = nextDb
        rows.sessions = [...nextDb.sessions.toArray]
        rows.promptRequests = [...nextDb.promptRequests.toArray]
        rows.permissions = [...nextDb.permissions.toArray]
        rows.chunks = [...nextDb.chunks.toArray]
        publish()

        subscriptions.push(
          nextDb.sessions.subscribe((nextRows) => {
            rows.sessions = [...nextRows]
            publish()
          }),
        )
        subscriptions.push(
          nextDb.promptRequests.subscribe((nextRows) => {
            rows.promptRequests = [...nextRows]
            publish()
          }),
        )
        subscriptions.push(
          nextDb.permissions.subscribe((nextRows) => {
            rows.permissions = [...nextRows]
            publish()
          }),
        )
        subscriptions.push(
          nextDb.chunks.subscribe((nextRows) => {
            rows.chunks = [...nextRows]
            publish()
          }),
        )
      })
      .catch((nextError: unknown) => {
        if (cancelled) {
          return
        }
        setError(nextError instanceof Error ? nextError.message : String(nextError))
        setStatus('error')
      })

    return () => {
      cancelled = true
      if (animationFrame !== 0) {
        window.cancelAnimationFrame(animationFrame)
      }
      for (const subscription of subscriptions) {
        subscription.unsubscribe()
      }
      db?.close()
    }
  }, [])

  if (status === 'error') {
    return h('main', { style: pageStyle },
      h('section', { style: shellStyle }, [
        h(
          'h1',
          {
            key: 'title',
            style: {
              fontFamily: 'Georgia, serif',
              fontSize: '48px',
              margin: '0 0 16px',
            },
          },
          'Fireline live monitoring',
        ),
        h(
          'p',
          {
            key: 'summary',
            style: {
              maxWidth: '720px',
              color: '#d7ccba',
              fontSize: '18px',
              lineHeight: '1.6',
              margin: '0 0 24px',
            },
          },
          'This dashboard only needs the state stream. If observation fails here, fix the stream URL before you debug anything else.',
        ),
        h(
          'div',
          {
            key: 'error',
            style: {
              ...cardStyle,
              border: '1px solid rgba(255, 136, 107, 0.45)',
              color: '#ffd2c4',
            },
          },
          [
            h('span', { key: 'label', style: labelStyle }, 'Connection error'),
            h(
              'code',
              {
                key: 'value',
                style: {
                  fontSize: '14px',
                  lineHeight: '1.6',
                  whiteSpace: 'pre-wrap',
                },
              },
              error ?? 'Unknown error',
            ),
          ],
        ),
      ]),
    )
  }

  return h(MonitoringView, { status, snapshot })
}

function MonitoringView({
  status,
  snapshot,
}: {
  status: 'connecting' | 'connected'
  snapshot: MonitorSnapshot
}) {
  return h('main', { style: pageStyle },
    h('section', { style: shellStyle }, [
      h('header', { key: 'header', style: { marginBottom: '32px' } }, [
        h(
          'div',
          { key: 'eyebrow', style: { ...labelStyle, marginBottom: '14px' } },
          'Reactive durable-state wallboard',
        ),
        h(
          'h1',
          {
            key: 'title',
            style: {
              fontFamily: 'Georgia, serif',
              fontSize: '54px',
              lineHeight: 1,
              margin: '0 0 18px',
            },
          },
          'See which agents are alive, blocked, or done without polling anything.',
        ),
        h(
          'p',
          {
            key: 'summary',
            style: {
              maxWidth: '760px',
              color: '#d7ccba',
              fontSize: '19px',
              lineHeight: '1.6',
              margin: '0 0 18px',
            },
          },
          'This page opens one Fireline state stream, subscribes to the current sessions, prompt requests, approvals, and chunks, and derives the operator view from those four collections.',
        ),
        h(
          'div',
          {
            key: 'stream',
            style: {
              display: 'inline-flex',
              flexWrap: 'wrap',
              gap: '12px',
              alignItems: 'center',
              padding: '10px 14px',
              borderRadius: '999px',
              background: 'rgba(255, 255, 255, 0.06)',
              border: '1px solid rgba(247, 240, 223, 0.12)',
              fontSize: '14px',
            },
          },
          [
            h(
              'strong',
              {
                key: 'status',
                style: { color: status === 'connected' ? '#97f0bf' : '#ffd78d' },
              },
              status === 'connected' ? 'Subscribed' : 'Connecting',
            ),
            h('code', { key: 'url', style: { color: '#f7f0df' } }, stateStreamUrl),
          ],
        ),
      ]),
      h(
        'section',
        { key: 'metrics', style: { ...gridStyle, marginBottom: '22px' } },
        [
          metricCard('Sessions', snapshot.sessions, `${snapshot.activeSessions} active right now`),
          metricCard(
            'Requests',
            snapshot.activeRequests,
            `${snapshot.queuedRequests} queued, ${snapshot.completedRequests} completed`,
          ),
          metricCard(
            'Approvals',
            snapshot.pendingApprovals,
            snapshot.pendingApprovals === 0
              ? 'No one is blocked on human input'
              : 'Human review needed',
          ),
          metricCard(
            'Tool Calls',
            snapshot.toolCalls,
            'Observed from durable chunk updates',
          ),
        ],
      ),
      h(
        'section',
        { key: 'activity', style: { ...cardStyle, marginBottom: '22px' } },
        [
          h('span', { key: 'label', style: labelStyle }, 'Latest visible work'),
          h(
            'div',
            { key: 'value', style: { fontSize: '20px', lineHeight: '1.5' } },
            snapshot.latestActivity,
          ),
        ],
      ),
      h('section', { key: 'cards' }, [
        h(
          'div',
          { key: 'section-label', style: { ...labelStyle, marginBottom: '16px' } },
          'Current sessions',
        ),
        snapshot.cards.length === 0
          ? h(
              'div',
              { key: 'empty', style: { ...cardStyle, color: '#d7ccba' } },
              'No sessions are present in this stream yet. Start any Fireline example, then point this dashboard at its stateStream URL.',
            )
          : h(
              'div',
              {
                key: 'grid',
                style: {
                  display: 'grid',
                  gap: '16px',
                  gridTemplateColumns: 'repeat(auto-fit, minmax(260px, 1fr))',
                },
              },
              snapshot.cards.map((card) =>
                h('article', { key: card.sessionId, style: cardStyle }, [
                  h(
                    'div',
                    {
                      key: 'meta',
                      style: {
                        display: 'flex',
                        justifyContent: 'space-between',
                        gap: '12px',
                        alignItems: 'center',
                        marginBottom: '16px',
                      },
                    },
                    [
                      h(
                        'code',
                        { key: 'id', style: { fontSize: '13px', color: '#f7f0df' } },
                        abbreviate(card.sessionId),
                      ),
                      h(
                        'span',
                        {
                          key: 'state',
                          style: stateBadgeStyle(card.state, card.pendingApprovals),
                        },
                        card.pendingApprovals > 0 ? 'awaiting approval' : card.state,
                      ),
                    ],
                  ),
                  h(
                    'div',
                    {
                      key: 'counts',
                      style: {
                        display: 'flex',
                        gap: '16px',
                        marginBottom: '16px',
                        fontSize: '14px',
                        color: '#d7ccba',
                      },
                    },
                    [
                      h('span', { key: 'requests' }, `${card.requestCount} requests`),
                      h('span', { key: 'active' }, `${card.activeRequests} active`),
                      h('span', { key: 'pending' }, `${card.pendingApprovals} pending`),
                    ],
                  ),
                  h('div', { key: 'activity-label', style: labelStyle }, 'Latest activity'),
                  h(
                    'div',
                    {
                      key: 'activity',
                      style: { fontSize: '16px', lineHeight: '1.5', minHeight: '72px' },
                    },
                    card.latestActivity,
                  ),
                  h(
                    'div',
                    {
                      key: 'updated',
                      style: { marginTop: '16px', fontSize: '13px', color: '#c7bda7' },
                    },
                    `Updated ${formatTime(card.updatedAt)}`,
                  ),
                ]),
              ),
            ),
      ]),
    ]),
  )
}

function metricCard(label: string, value: number, detail: string) {
  return h('div', { key: label, style: cardStyle }, [
    h('span', { key: 'label', style: labelStyle }, label),
    h('span', { key: 'value', style: valueStyle }, String(value)),
    h(
      'span',
      {
        key: 'detail',
        style: {
          color: '#d7ccba',
          fontSize: '14px',
          lineHeight: '1.5',
          marginTop: '10px',
          display: 'block',
        },
      },
      detail,
    ),
  ])
}

function buildSnapshot(rows: MonitorRows): MonitorSnapshot {
  const promptRequestsBySession = groupBy(rows.promptRequests, (row) => row.sessionId)
  const permissionsBySession = groupBy(rows.permissions, (row) => row.sessionId)
  const chunksBySession = groupBy(rows.chunks, (row) => row.sessionId)

  const cards = [...rows.sessions]
    .sort((left, right) => right.lastSeenAt - left.lastSeenAt)
    .map((session) => {
      const promptRequests = promptRequestsBySession.get(session.sessionId) ?? []
      const permissions = permissionsBySession.get(session.sessionId) ?? []
      const chunks = chunksBySession.get(session.sessionId) ?? []
      const pendingApprovals = permissions.filter((row) => row.state === 'pending').length
      const activeRequests = promptRequests.filter((row) => row.state === 'active').length
      const latestChunk = [...chunks]
        .sort((left, right) => right.createdAt - left.createdAt)
        .find((row) => extractChunkTextPreview(row.update).trim().length > 0)
      const latestRequest = [...promptRequests]
        .sort((left, right) => right.startedAt - left.startedAt)[0]
      const latestActivity = truncate(
        latestChunk
          ? extractChunkTextPreview(latestChunk.update).trim()
          : latestRequest?.text?.trim() || 'Waiting for the next update.',
      )
      const updatedAt =
        latestChunk?.createdAt ?? latestRequest?.startedAt ?? session.lastSeenAt

      return {
        sessionId: session.sessionId,
        state: session.state,
        requestCount: promptRequests.length,
        activeRequests,
        pendingApprovals,
        latestActivity,
        updatedAt,
      }
    })

  const latestVisibleChunk = [...rows.chunks]
    .sort((left, right) => right.createdAt - left.createdAt)
    .find((row) => extractChunkTextPreview(row.update).trim().length > 0)
  const latestVisibleRequest = [...rows.promptRequests]
    .sort((left, right) => right.startedAt - left.startedAt)[0]

  return {
    sessions: rows.sessions.length,
    activeSessions: rows.sessions.filter((row) => row.state === 'active').length,
    activeRequests: rows.promptRequests.filter((row) => row.state === 'active').length,
    queuedRequests: rows.promptRequests.filter((row) => row.state === 'queued').length,
    completedRequests: rows.promptRequests.filter((row) => row.state === 'completed').length,
    pendingApprovals: rows.permissions.filter((row) => row.state === 'pending').length,
    toolCalls: rows.chunks.filter((row) => isToolCallSessionUpdate(row.update)).length,
    latestActivity: truncate(
      latestVisibleChunk
        ? extractChunkTextPreview(latestVisibleChunk.update).trim()
        : latestVisibleRequest?.text?.trim() || 'Waiting for the first durable event.',
    ),
    cards,
  }
}

function groupBy<T>(rows: T[], key: (row: T) => string): Map<string, T[]> {
  const groups = new Map<string, T[]>()
  for (const row of rows) {
    const groupKey = key(row)
    const current = groups.get(groupKey)
    if (current) {
      current.push(row)
      continue
    }
    groups.set(groupKey, [row])
  }
  return groups
}

function abbreviate(value: string): string {
  return value.length <= 20 ? value : `${value.slice(0, 8)}...${value.slice(-8)}`
}

function truncate(value: string, max = 160): string {
  return value.length <= max ? value : `${value.slice(0, max - 3)}...`
}

function formatTime(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString([], {
    hour: 'numeric',
    minute: '2-digit',
    second: '2-digit',
  })
}

function stateBadgeStyle(
  state: SessionRow['state'],
  pendingApprovals: number,
): CSSProperties {
  const background = pendingApprovals > 0
    ? 'rgba(255, 197, 102, 0.18)'
    : state === 'active'
      ? 'rgba(114, 236, 165, 0.18)'
      : state === 'broken'
        ? 'rgba(255, 107, 107, 0.18)'
        : 'rgba(247, 240, 223, 0.12)'
  const color = pendingApprovals > 0
    ? '#ffd78d'
    : state === 'active'
      ? '#97f0bf'
      : state === 'broken'
        ? '#ffb3b3'
        : '#f7f0df'

  return {
    alignSelf: 'flex-start',
    padding: '6px 10px',
    borderRadius: '999px',
    background,
    border: '1px solid rgba(247, 240, 223, 0.12)',
    color,
    fontSize: '12px',
    letterSpacing: '0.08em',
    textTransform: 'uppercase',
  }
}

createRoot(document.getElementById('app')!).render(h(App))
