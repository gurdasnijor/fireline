import { Box, Text } from 'ink'
import React, { useEffect, useMemo, useState } from 'react'
import type {
  ChunkRow,
  PermissionRow,
  PromptRequestRow,
  SessionRow,
} from '@fireline/state'
import type { FirelineDB } from '@fireline/client'
import {
  extractChunkTextPreview,
  sessionUpdateKind,
  sessionUpdateStatus,
  sessionUpdateTitle,
} from '@fireline/state'
import { REPL_PALETTE } from './repl-palette.js'

export interface SessionStatePaneProps {
  readonly db: FirelineDB | null
  readonly sessionId: string | null
  readonly runtimeId?: string | null
  readonly acpUrl?: string | null
  readonly serverUrl?: string | null
  readonly stateStreamUrl?: string | null
}

export function SessionStatePane(props: SessionStatePaneProps) {
  const sessions = useCollectionRows(props.db?.sessions ?? null)
  const promptRequests = useCollectionRows(props.db?.promptRequests ?? null)
  const permissions = useCollectionRows(props.db?.permissions ?? null)
  const chunks = useCollectionRows(props.db?.chunks ?? null)

  const sessionRow = useMemo(
    () =>
      props.sessionId
        ? sessions.find((row) => row.sessionId === props.sessionId) ?? null
        : null,
    [props.sessionId, sessions],
  )

  const sessionTurns = useMemo(
    () =>
      props.sessionId
        ? promptRequests
            .filter((row) => row.sessionId === props.sessionId)
            .sort((left, right) => right.startedAt - left.startedAt)
        : [],
    [props.sessionId, promptRequests],
  )

  const sessionPermissions = useMemo(
    () =>
      props.sessionId
        ? permissions
            .filter((row) => row.sessionId === props.sessionId)
            .sort(permissionSort)
        : [],
    [permissions, props.sessionId],
  )

  const chunkSummaries = useMemo(
    () => summarizeChunksByRequest(chunks, props.sessionId),
    [chunks, props.sessionId],
  )

  return (
    <Box flexDirection="column">
      {/* Data source: db.sessions.subscribe(...) for the selected session row.
          This card also includes operator-plane attachment metadata passed in
          from the REPL host because runtime/endpoint facts do not belong in
          fireline.db() itself. */}
      <SessionMetaCard
        acpUrl={props.acpUrl ?? null}
        runtimeId={props.runtimeId ?? null}
        serverUrl={props.serverUrl ?? null}
        sessionId={props.sessionId}
        sessionRow={sessionRow}
        stateStreamUrl={props.stateStreamUrl ?? null}
      />

      {/* Data source: db.promptRequests.subscribe(...) narrowed to the current
          session. Chunk counts and latest chunk previews are derived from
          db.chunks.subscribe(...) and grouped by request id. */}
      {sessionTurns.length > 0 ? (
        sessionTurns.map((turn) => (
          <PromptTurnCard
            key={requestKey(turn)}
            summary={chunkSummaries.get(requestKey(turn)) ?? EMPTY_CHUNK_SUMMARY}
            turn={turn}
          />
        ))
      ) : (
        <EmptyCard
          title="Prompt turns"
          message={
            props.sessionId
              ? 'No durable prompt-turn rows exist for this session yet.'
              : 'Attach to a session to materialize prompt-turn state.'
          }
        />
      )}

      {/* Data source: db.permissions.subscribe(...) narrowed to the current
          session. Pending and resolved rows share one card language so pane 2
          stays a read model instead of an event log. */}
      {sessionPermissions.length > 0 ? (
        sessionPermissions.map((permission) => (
          <PermissionCard key={requestKey(permission)} permission={permission} />
        ))
      ) : (
        <EmptyCard
          title="Approval state"
          message={
            props.sessionId
              ? 'No permission rows have been materialized for this session.'
              : 'Attach to a session to watch approval state.'
          }
        />
      )}
    </Box>
  )
}

function SessionMetaCard(props: {
  readonly acpUrl: string | null
  readonly runtimeId: string | null
  readonly serverUrl: string | null
  readonly sessionId: string | null
  readonly sessionRow: SessionRow | null
  readonly stateStreamUrl: string | null
}) {
  return (
    <Card borderColor={REPL_PALETTE.assistant} title="Session state">
      <MetadataLine label="session" value={props.sessionId ?? 'connecting'} />
      <MetadataLine
        dim
        label="status"
        value={props.sessionRow?.state ?? 'waiting for durable session row'}
      />
      <MetadataLine
        dim
        label="last seen"
        value={formatTimestamp(props.sessionRow?.lastSeenAt)}
      />
      <MetadataLine
        dim
        label="load"
        value={props.sessionRow ? (props.sessionRow.supportsLoadSession ? 'supported' : 'not advertised') : 'unknown'}
      />
      <MetadataLine label="runtime" value={props.runtimeId ?? 'unknown'} />
      <MetadataLine dim label="host" value={hostLabel(props.serverUrl)} />
      <MetadataLine dim label="acp" value={endpointLabel(props.acpUrl)} />
      <MetadataLine dim label="stream" value={endpointLabel(props.stateStreamUrl)} />
      <MetadataLine dim label="spawn" value="not yet surfaced in REPL state" />
    </Card>
  )
}

function PromptTurnCard(props: {
  readonly summary: ChunkSummary
  readonly turn: PromptRequestRow
}) {
  const stateColor = promptTurnColor(props.turn.state)
  const title = promptTurnTitle(props.turn)

  return (
    <Card borderColor={stateColor} title={`Prompt turn · ${title}`}>
      <MetadataLine label="request" value={String(props.turn.requestId)} />
      <MetadataLine dim label="state" value={props.turn.state} />
      <MetadataLine
        dim
        label="started"
        value={formatTimestamp(props.turn.startedAt)}
      />
      <MetadataLine
        dim
        label="completed"
        value={formatTimestamp(props.turn.completedAt)}
      />
      {props.turn.stopReason ? (
        <MetadataLine dim label="stop" value={props.turn.stopReason} />
      ) : null}
      <Box marginTop={1}>
        <Text>{summarizePromptText(props.turn.text)}</Text>
      </Box>
      <Box marginTop={1} flexDirection="column">
        <Text color={REPL_PALETTE.subdued}>chunk summary</Text>
        <Text>
          total {props.summary.total}  tools {props.summary.toolUpdates}  latest{' '}
          {props.summary.latestKind ?? 'none'}
        </Text>
        <Text color={REPL_PALETTE.subdued} dimColor>
          {props.summary.latestPreview ?? 'no chunk preview yet'}
        </Text>
      </Box>
    </Card>
  )
}

function PermissionCard(props: { readonly permission: PermissionRow }) {
  const borderColor = permissionColor(props.permission)
  const outcome = permissionOutcomeLabel(props.permission)

  return (
    <Card borderColor={borderColor} title={`Approval · ${outcome}`}>
      <MetadataLine label="request" value={String(props.permission.requestId)} />
      {props.permission.toolCallId ? (
        <MetadataLine dim label="tool" value={props.permission.toolCallId} />
      ) : null}
      <MetadataLine dim label="state" value={props.permission.state} />
      {props.permission.outcome ? (
        <MetadataLine dim label="outcome" value={props.permission.outcome} />
      ) : null}
      <MetadataLine
        dim
        label="created"
        value={formatTimestamp(props.permission.createdAt)}
      />
      <MetadataLine
        dim
        label="resolved"
        value={formatTimestamp(props.permission.resolvedAt)}
      />
      <Box marginTop={1}>
        <Text>{props.permission.title ?? 'No approval reason was recorded.'}</Text>
      </Box>
    </Card>
  )
}

function EmptyCard(props: { readonly title: string; readonly message: string }) {
  return (
    <Card borderColor={REPL_PALETTE.subdued} title={props.title}>
      <Text color={REPL_PALETTE.subdued}>{props.message}</Text>
    </Card>
  )
}

function Card(props: {
  readonly borderColor: string
  readonly children: React.ReactNode
  readonly title: string
}) {
  return (
    <Box
      borderColor={props.borderColor}
      borderStyle="round"
      flexDirection="column"
      marginBottom={1}
      paddingX={1}
    >
      <Text bold color={props.borderColor}>
        {props.title}
      </Text>
      <Box flexDirection="column">{props.children}</Box>
    </Box>
  )
}

function MetadataLine(props: {
  readonly dim?: boolean
  readonly label: string
  readonly value: string
}) {
  return (
    <Text color={props.dim ? REPL_PALETTE.subdued : undefined} dimColor={props.dim}>
      {props.label} {truncateMiddle(props.value, 44)}
    </Text>
  )
}

type ChunkSummary = {
  readonly latestKind: string | null
  readonly latestPreview: string | null
  readonly total: number
  readonly toolUpdates: number
}

const EMPTY_CHUNK_SUMMARY: ChunkSummary = {
  latestKind: null,
  latestPreview: null,
  total: 0,
  toolUpdates: 0,
}

function useCollectionRows<T extends object>(
  collection:
    | {
        readonly toArray: readonly T[]
        readonly subscribe: (callback: (rows: T[]) => void) => { unsubscribe(): void }
      }
    | null,
): readonly T[] {
  const [rows, setRows] = useState<readonly T[]>(collection?.toArray ?? [])

  useEffect(() => {
    if (!collection) {
      setRows([])
      return
    }

    const subscription = collection.subscribe((nextRows) => {
      setRows([...nextRows])
    })

    return () => {
      subscription.unsubscribe()
    }
  }, [collection])

  return rows
}

function summarizeChunksByRequest(
  rows: readonly ChunkRow[],
  sessionId: string | null,
): Map<string, ChunkSummary> {
  const byRequest = new Map<string, ChunkSummary>()

  for (const row of rows) {
    if (sessionId && row.sessionId !== sessionId) {
      continue
    }

    const key = requestKey(row)
    const next: ChunkSummary = byRequest.get(key) ?? EMPTY_CHUNK_SUMMARY
    byRequest.set(key, {
      latestKind: sessionUpdateKind(row.update) || next.latestKind,
      latestPreview: latestPreview(row) ?? next.latestPreview,
      total: next.total + 1,
      toolUpdates:
        next.toolUpdates + (row.toolCallId || sessionUpdateStatus(row.update) ? 1 : 0),
    })
  }

  return byRequest
}

function latestPreview(row: ChunkRow): string | null {
  const preview = extractChunkTextPreview(row.update)
  if (preview) {
    return preview
  }

  const title = sessionUpdateTitle(row.update)
  if (title) {
    return title
  }

  const status = sessionUpdateStatus(row.update)
  return status ?? null
}

function requestKey(
  row: Pick<PromptRequestRow, 'sessionId' | 'requestId'>,
): string {
  return `${row.sessionId}:${String(row.requestId)}`
}

function permissionSort(left: PermissionRow, right: PermissionRow): number {
  const leftRank = permissionRank(left)
  const rightRank = permissionRank(right)
  if (leftRank !== rightRank) {
    return leftRank - rightRank
  }
  return (right.resolvedAt ?? right.createdAt) - (left.resolvedAt ?? left.createdAt)
}

function permissionRank(permission: PermissionRow): number {
  if (permission.state === 'pending') {
    return 0
  }
  if (permission.outcome === 'approved') {
    return 1
  }
  if (permission.outcome === 'denied') {
    return 2
  }
  return 3
}

function permissionColor(permission: PermissionRow): string {
  if (permission.state === 'pending') {
    return REPL_PALETTE.pending
  }
  if (permission.outcome === 'approved') {
    return REPL_PALETTE.resolvedAllow
  }
  if (permission.outcome === 'denied') {
    return REPL_PALETTE.resolvedDeny
  }
  return REPL_PALETTE.subdued
}

function permissionOutcomeLabel(permission: PermissionRow): string {
  if (permission.state === 'pending') {
    return 'pending'
  }
  if (permission.outcome === 'approved') {
    return 'allowed'
  }
  if (permission.outcome === 'denied') {
    return 'denied'
  }
  return permission.state
}

function promptTurnColor(state: PromptRequestRow['state']): string {
  switch (state) {
    case 'active':
      return REPL_PALETTE.streaming
    case 'queued':
    case 'cancel_requested':
      return REPL_PALETTE.pending
    case 'completed':
      return REPL_PALETTE.resolvedAllow
    case 'broken':
    case 'timed_out':
    case 'cancelled':
      return REPL_PALETTE.resolvedDeny
    default:
      return REPL_PALETTE.subdued
  }
}

function promptTurnTitle(turn: PromptRequestRow): string {
  return turn.position !== undefined ? `#${turn.position}` : String(turn.requestId)
}

function summarizePromptText(text: string | undefined): string {
  if (!text) {
    return 'No prompt text captured in the durable row.'
  }

  return truncateMiddle(text.replace(/\s+/g, ' ').trim(), 88)
}

function formatTimestamp(value: number | undefined): string {
  if (!value) {
    return 'n/a'
  }

  return new Date(value).toISOString()
}

function hostLabel(value: string | null): string {
  if (!value) {
    return 'unknown'
  }
  try {
    return new URL(value).host
  } catch {
    return value
  }
}

function endpointLabel(value: string | null): string {
  if (!value) {
    return 'unknown'
  }
  try {
    const url = new URL(value)
    const tail = url.pathname.split('/').filter(Boolean).at(-1) ?? url.pathname
    return `${url.host}/${truncateMiddle(tail, 24)}`
  } catch {
    return truncateMiddle(value, 36)
  }
}

function truncateMiddle(value: string, maxLength: number): string {
  if (value.length <= maxLength) {
    return value
  }

  const side = Math.max(6, Math.floor((maxLength - 1) / 2))
  return `${value.slice(0, side)}…${value.slice(-side)}`
}
