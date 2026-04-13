import assert from 'node:assert/strict'
import test from 'node:test'
import { renderToString } from 'ink'
import React from 'react'
import {
  ConversationPane,
  type ConversationPaneProps,
} from './repl-pane-conversation.js'
import type {
  PendingApproval,
  ReplViewState,
  TranscriptEntry,
} from './repl.js'

function makeState(overrides: Partial<ReplViewState> = {}): ReplViewState {
  return {
    acpUrl: 'ws://127.0.0.1:4440/acp',
    busy: false,
    entries: [],
    pendingApproval: null,
    pendingTools: 0,
    resolvingApproval: false,
    runtimeId: 'runtime:demo-runtime',
    serverUrl: 'http://127.0.0.1:4440',
    sessionId: 'session-123',
    stateStreamUrl: 'http://127.0.0.1:7474/v1/stream/demo-stream',
    usage: null,
    ...overrides,
  }
}

function renderPane(overrides: Partial<ConversationPaneProps> = {}) {
  const committedEntries: readonly TranscriptEntry[] = overrides.committedEntries ?? []
  const liveEntries: readonly TranscriptEntry[] = overrides.liveEntries ?? []
  const state = overrides.state ?? makeState({
    entries: [...committedEntries, ...liveEntries],
  })

  return renderToString(
    React.createElement(ConversationPane, {
      committedEntries,
      focusedApprovalAction: overrides.focusedApprovalAction ?? 'allow',
      input: overrides.input ?? '',
      liveEntries,
      spinner: overrides.spinner ?? 'o',
      state,
      title: overrides.title,
    }),
    { columns: 100 },
  )
}

test('conversation pane renders committed scrollback and live shell content together', () => {
  const committedEntries: readonly TranscriptEntry[] = [
    { id: 1, kind: 'message', role: 'assistant', text: 'startup banner' },
  ]
  const liveEntries: readonly TranscriptEntry[] = [
    { id: 2, kind: 'message', role: 'user', text: 'investigate the failing build' },
    { id: 3, kind: 'tool', detail: 'src/index.ts', status: 'pending', title: 'edit file', toolCallId: 'tool-1', toolKind: 'edit' },
  ]

  const output = renderPane({
    committedEntries,
    liveEntries,
    input: 'follow-up prompt',
  })

  assert.match(output, /Conversation/)
  assert.match(output, /startup banner/)
  assert.match(output, /investigate the failing build/)
  assert.match(output, /tool pending/)
  assert.match(output, /follow-up prompt/)
})

test('conversation pane explains scrollback when the active turn is idle', () => {
  const committedEntries: readonly TranscriptEntry[] = [
    { id: 1, kind: 'message', role: 'assistant', text: 'completed reply' },
  ]

  const output = renderPane({ committedEntries })

  assert.match(output, /completed reply/)
  assert.match(output, /Earlier conversation lives in terminal scrollback\./)
  assert.match(output, /Type a new prompt below to continue the session\./)
})

test('conversation pane renders the approval card inside the live shell', () => {
  const pendingApproval: PendingApproval = {
    requestId: 'request-123',
    reason: 'awaiting durable approval resolution',
    sessionId: 'session-123',
    summary: 'Write test.txt to /workspace',
    toolCallId: 'tool-1',
  }
  const entries: readonly TranscriptEntry[] = [
    { id: 1, kind: 'message', role: 'user', text: '{"command":"write_file","path":"/workspace/test.txt","content":"hello"}' },
    { id: 2, kind: 'tool', detail: 'write_file /workspace/test.txt', status: 'pending', title: 'write_file', toolCallId: 'tool-1', toolKind: 'edit' },
  ]

  const output = renderPane({
    liveEntries: entries,
    state: makeState({
      entries,
      pendingApproval,
      pendingTools: 1,
    }),
  })

  assert.match(output, /Tool Approval/)
  assert.match(output, /write_file/)
  assert.match(output, /"path": "\/workspace\/test\.txt"/)
  assert.match(output, /awaiting durable approval resolution/)
  assert.match(output, /Accept/)
  assert.match(output, /Decline/)
})
