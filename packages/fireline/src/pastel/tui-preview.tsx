import React, { useMemo } from 'react'
import { Box, Text } from 'ink'
import type { FirelineDB } from '@fireline/client'
import type {
  ChunkRow,
  PermissionRow,
  PromptRequestRow,
  SessionRow,
} from '@fireline/state'
import {
  EventStreamPane,
  EventStreamStore,
} from '../repl-pane-events.js'
import { REPL_PALETTE } from '../repl-palette.js'
import { ConversationPane } from '../repl-pane-conversation.js'
import { SessionStatePane } from '../repl-pane-state.js'
import type {
  ReplViewState,
  TranscriptEntry,
} from '../repl.js'

export interface FirelineTuiPreviewProps {
  readonly sessionId?: string
}

export function FirelineTuiPreview(props: FirelineTuiPreviewProps) {
  const sessionId = props.sessionId ?? 'session-preview'
  const acpUrl = 'ws://127.0.0.1:4440/acp'
  const serverUrl = 'http://127.0.0.1:4440'
  const runtimeId = 'runtime:preview-runtime'
  const stateStreamUrl = `http://127.0.0.1:7474/v1/stream/fireline-state-${sessionId}`
  const transcript = useMemo(() => createPreviewTranscript(), [])
  const viewState = useMemo<ReplViewState>(() => ({
    acpUrl,
    busy: true,
    entries: transcript,
    pendingApproval: {
      requestId: 'req-2',
      reason: 'durable approval pending for write_file',
      sessionId,
      summary: 'Write deploy/fly.toml',
      toolCallId: 'tool-2',
    },
    pendingTools: 1,
    resolvingApproval: false,
    runtimeId,
    serverUrl,
    sessionId,
    stateStreamUrl,
    usage: {
      cost: null,
      size: 8192,
      used: 2650,
    },
  }), [acpUrl, runtimeId, serverUrl, sessionId, stateStreamUrl, transcript])
  const panes = useMemo(() => partitionForPreview(viewState.entries), [viewState.entries])
  const db = useMemo(() => createPreviewDb(sessionId), [sessionId])
  const eventController = useMemo(() => createPreviewEventStore(sessionId), [sessionId])

  return (
    <Box flexDirection="row">
      <Box flexDirection="column" flexGrow={5} flexShrink={1} marginRight={1}>
        <ConversationPane
          committedEntries={panes.committedEntries}
          focusedApprovalAction="allow"
          input="show me the restart-safe approval flow"
          liveEntries={panes.liveEntries}
          spinner="o"
          state={viewState}
          title="Pane 1 · Conversation"
        />
      </Box>
      <Box flexDirection="column" flexGrow={3} flexShrink={1}>
        <Text bold color={REPL_PALETTE.assistant}>
          Pane 2 · Materialized state
        </Text>
        <SessionStatePane
          acpUrl={acpUrl}
          db={db}
          runtimeId={runtimeId}
          serverUrl={serverUrl}
          sessionId={sessionId}
          stateStreamUrl={stateStreamUrl}
        />
        <Box marginTop={1}>
          <EventStreamPane
            controller={eventController}
            focused={false}
            maxVisibleRows={8}
            title="Pane 3 · Realtime events"
          />
        </Box>
      </Box>
    </Box>
  )
}

function createPreviewTranscript(): readonly TranscriptEntry[] {
  return [
    {
      id: 1,
      kind: 'message',
      role: 'assistant',
      text: 'Booted Fireline preview shell. Durable state attached and ACP ready.',
    },
    {
      id: 2,
      kind: 'message',
      role: 'user',
      text: 'Deploy the hosted image and capture the approval workflow.',
    },
    {
      id: 3,
      kind: 'plan',
      items: [
        'Inspect the deployment target',
        'Build the OCI image',
        'Capture approval + restart evidence',
      ],
    },
    {
      id: 4,
      kind: 'tool',
      detail: 'docker build -t fireline-preview .',
      status: 'completed',
      title: 'Build OCI image',
      toolCallId: 'tool-1',
      toolKind: 'shell',
    },
    {
      id: 5,
      kind: 'message',
      role: 'assistant',
      text: 'The image built cleanly. I am now preparing the approval checkpoint.',
    },
    {
      id: 6,
      kind: 'message',
      role: 'user',
      text: '{"command":"write_file","path":"deploy/fly.toml","content":"app = \\"fireline-demo\\""}',
    },
    {
      id: 7,
      kind: 'tool',
      detail: 'write_file deploy/fly.toml',
      status: 'pending',
      title: 'write_file',
      toolCallId: 'tool-2',
      toolKind: 'edit',
    },
    {
      id: 8,
      kind: 'message',
      role: 'assistant',
      text: 'Approval is pending. Once allowed, I will write the target scaffold and continue.',
    },
  ]
}

function partitionForPreview(entries: readonly TranscriptEntry[]): {
  readonly committedEntries: readonly TranscriptEntry[]
  readonly liveEntries: readonly TranscriptEntry[]
} {
  return {
    committedEntries: entries.slice(0, 5),
    liveEntries: entries.slice(5),
  }
}

function createPreviewEventStore(sessionId: string): EventStreamStore {
  const store = new EventStreamStore()
  store.noteControlSurfaceGap(
    'Host lifecycle events still use a preview stub until the control-plane bus lands.',
  )
  store.append({
    timestamp: 1_710_000_000_001,
    source: 'control',
    name: 'host.boot',
    payload: 'dir=internal runtime=preview-runtime',
    sessionId,
  })
  store.append({
    timestamp: 1_710_000_000_020,
    source: 'acp',
    name: 'session/new',
    payload: `dir=out cwd=/workspace session=${sessionId}`,
    requestId: 'req-1',
    sessionId,
  })
  store.append({
    timestamp: 1_710_000_000_040,
    source: 'durable',
    name: 'promptTurns.append',
    payload: `op=append key=${sessionId}:req-2 session=${sessionId} request=req-2 state=active`,
    requestId: 'req-2',
    sessionId,
  })
  store.append({
    timestamp: 1_710_000_000_060,
    source: 'durable',
    name: 'permissions.append',
    payload: `op=append key=${sessionId}:req-2 session=${sessionId} request=req-2 kind=permission_request state=pending`,
    requestId: 'req-2',
    sessionId,
  })
  store.append({
    timestamp: 1_710_000_000_080,
    source: 'acp',
    name: 'session_update',
    payload: 'dir=in kind=agent_message_chunk text=Approval is pending',
    requestId: 'req-2',
    sessionId,
  })
  return store
}

function createPreviewDb(sessionId: string): FirelineDB {
  const sessions: readonly SessionRow[] = [
    {
      sessionId,
      state: 'active',
      supportsLoadSession: true,
      createdAt: 1_710_000_000_000,
      updatedAt: 1_710_000_000_120,
      lastSeenAt: 1_710_000_000_120,
    },
  ]
  const promptRequests: readonly PromptRequestRow[] = [
    {
      sessionId,
      requestId: 'req-1',
      text: 'Deploy the hosted image and capture the approval workflow.',
      state: 'completed',
      startedAt: 1_710_000_000_010,
      completedAt: 1_710_000_000_040,
      stopReason: 'end_turn',
    },
    {
      sessionId,
      requestId: 'req-2',
      text: 'Write deploy/fly.toml once the operator approves it.',
      state: 'active',
      startedAt: 1_710_000_000_050,
    },
  ]
  const permissions: readonly PermissionRow[] = [
    {
      sessionId,
      requestId: 'req-2',
      title: 'Write deploy/fly.toml',
      toolCallId: 'tool-2',
      state: 'pending',
      createdAt: 1_710_000_000_060,
    },
    {
      sessionId,
      requestId: 'req-1',
      title: 'Push preview branch',
      toolCallId: 'tool-1',
      state: 'resolved',
      outcome: 'approved',
      createdAt: 1_710_000_000_020,
      resolvedAt: 1_710_000_000_025,
    },
  ]
  const chunks: readonly ChunkRow[] = [
    {
      sessionId,
      requestId: 'req-1',
      createdAt: 1_710_000_000_030,
      update: {
        content: {
          text: 'The image built cleanly.',
          type: 'text',
        },
        sessionUpdate: 'agent_message_chunk',
      },
    },
    {
      sessionId,
      requestId: 'req-2',
      toolCallId: 'tool-2',
      createdAt: 1_710_000_000_070,
      update: {
        sessionUpdate: 'tool_call',
        status: 'pending',
        title: 'Write deploy/fly.toml',
        toolCallId: 'tool-2',
      },
    },
  ]

  return {
    sessions: createFakeCollection(sessions),
    promptRequests: createFakeCollection(promptRequests),
    permissions: createFakeCollection(permissions),
    chunks: createFakeCollection(chunks),
    collections: {
      sessions: createFakeCollection(sessions),
      promptRequests: createFakeCollection(promptRequests),
      permissions: createFakeCollection(permissions),
      chunks: createFakeCollection(chunks),
    },
    close() {},
    preload: async () => {},
    stream: {} as FirelineDB['stream'],
    utils: {} as FirelineDB['utils'],
  } as FirelineDB
}

function createFakeCollection<T extends object>(seed: readonly T[]) {
  const rows = [...seed]
  return {
    toArray: rows,
    subscribe(callback: (nextRows: T[]) => void) {
      callback(rows)
      return {
        unsubscribe() {},
      }
    },
  }
}
