import assert from 'node:assert/strict'
import test from 'node:test'
import { renderToString } from 'ink'
import React from 'react'
import type { FirelineDB } from '@fireline/client'
import type {
  ChunkRow,
  PermissionRow,
  PromptRequestRow,
  SessionRow,
} from '@fireline/state'
import { FirelineReplApp, partitionTranscriptEntries } from './repl-ui.js'
import { SessionStatePane } from './repl-pane-state.js'
import { ReplController } from './repl.js'

test('repl controller submits prompts and renders streamed output', async () => {
  const prompts: string[] = []

  const controller = new ReplController({
    acpUrl: 'ws://127.0.0.1:55371/acp',
    runtimeId: 'runtime:46ae8df5-5588-482c-a1ea-c85b1b49723d',
    resolveApproval: async () => {},
    sendPrompt: async (text) => {
      prompts.push(text)
      controller.receiveNotification({
        sessionId: 'session-123',
        update: {
          sessionUpdate: 'agent_message_chunk',
          content: {
            type: 'text',
            text: 'Hello back from Fireline.',
          },
        },
      })
    },
    serverUrl: 'http://127.0.0.1:4440',
    sessionId: 'session-123',
    stateStreamUrl: 'http://127.0.0.1:7474/v1/stream/fireline-state-runtime-46ae8df5-5588-482c-a1ea-c85b1b49723d',
  })

  const result = await controller.submit('Ping the host')
  const output = renderToString(
    React.createElement(FirelineReplApp, {
      controller,
      onExitRequest: (_code: number) => {},
      onFailure: (error: Error) => {
        throw error
      },
    }),
    { columns: 80 },
  )

  assert.equal(result, 'sent')
  assert.deepEqual(prompts, ['Ping the host'])
  assert.match(output, /Ping the host/)
  assert.match(output, /assistant/i)
  assert.match(output, /Hello back from Fireline\./)
  assert.match(output, /session:session-123/)
  assert.match(output, /runtime:46ae8df5/)
  assert.match(output, /acp:55371/)
})

test('repl controller surfaces pending approvals and resolves them', async () => {
  const approvals: Array<{ allow: boolean; requestId: string | number; sessionId: string }> = []

  const controller = new ReplController({
    resolveApproval: async (approval, allow) => {
      approvals.push({
        allow,
        requestId: approval.requestId,
        sessionId: approval.sessionId,
      })
    },
    sendPrompt: async (_text) => {},
    serverUrl: 'http://127.0.0.1:4440',
    sessionId: 'session-123',
  })

  await controller.submit('{"command":"write_file","path":"/workspace/test.txt","content":"hello"}')
  controller.setPendingApproval({
    requestId: 'request-123',
    reason: 'approval fallback: prompt-level gate until tool-call interception lands',
    sessionId: 'session-123',
    summary: 'Write test.txt to /workspace',
    toolCallId: null,
  })

  const output = renderToString(
    React.createElement(FirelineReplApp, {
      controller,
      onExitRequest: (_code: number) => {},
      onFailure: (error: Error) => {
        throw error
      },
    }),
    { columns: 80 },
  )

  assert.match(output, /Tool Approval/i)
  assert.match(output, /write_file/)
  assert.match(output, /"path": "\/workspace\/test\.txt"/)
  assert.match(output, /approval fallback: prompt-level gate until tool-call interception/i)
  assert.match(output, /lands/i)
  assert.match(output, /Accept/)
  assert.match(output, /Decline/)
  assert.match(output, /Composer is locked; resolve the approval card first\./)

  await controller.resolvePendingApproval(true)

  assert.deepEqual(approvals, [
    {
      allow: true,
      requestId: 'request-123',
      sessionId: 'session-123',
    },
  ])
  assert.equal(controller.getSnapshot().pendingApproval, null)
})

test('partitionTranscriptEntries commits completed turns and keeps the active turn live', () => {
  const transcript = [
    { id: 1, kind: 'message', role: 'assistant', text: 'startup banner' },
    { id: 2, kind: 'message', role: 'user', text: 'turn one' },
    { id: 3, kind: 'message', role: 'assistant', text: 'done' },
    { id: 4, kind: 'message', role: 'user', text: 'turn two' },
    { id: 5, kind: 'tool', toolCallId: 'tool-1', title: 'write file', status: 'pending', toolKind: 'edit', detail: null },
  ] as const

  const active = partitionTranscriptEntries({
    acpUrl: 'ws://127.0.0.1:4440/acp',
    busy: true,
    entries: transcript,
    pendingApproval: null,
    pendingTools: 1,
    resolvingApproval: false,
    runtimeId: 'runtime:demo',
    serverUrl: 'http://127.0.0.1:4440',
    sessionId: 'session-123',
    stateStreamUrl: 'http://127.0.0.1:7474/v1/stream/demo',
    usage: null,
  })

  assert.deepEqual(
    active.committedEntries.map((entry) => entry.id),
    [1, 2, 3],
  )
  assert.deepEqual(
    active.liveEntries.map((entry) => entry.id),
    [4, 5],
  )

  const idle = partitionTranscriptEntries({
    acpUrl: 'ws://127.0.0.1:4440/acp',
    busy: false,
    entries: transcript,
    pendingApproval: null,
    pendingTools: 0,
    resolvingApproval: false,
    runtimeId: 'runtime:demo',
    serverUrl: 'http://127.0.0.1:4440',
    sessionId: 'session-123',
    stateStreamUrl: 'http://127.0.0.1:7474/v1/stream/demo',
    usage: null,
  })

  assert.deepEqual(
    idle.committedEntries.map((entry) => entry.id),
    [1, 2, 3, 4, 5],
  )
  assert.deepEqual(idle.liveEntries, [])
})

test('session state pane renders durable session, prompt, permission, and chunk summaries', () => {
  const db = createFakeDb({
    sessions: [
      {
        sessionId: 'session-123',
        state: 'active',
        supportsLoadSession: true,
        createdAt: 1,
        updatedAt: 2,
        lastSeenAt: 3,
      },
    ],
    promptRequests: [
      {
        sessionId: 'session-123',
        requestId: 'req-1',
        text: 'Investigate the failing build and propose a fix.',
        state: 'active',
        position: 1,
        startedAt: 10,
      },
    ],
    permissions: [
      {
        sessionId: 'session-123',
        requestId: 'req-1',
        title: 'Edit src/index.ts',
        toolCallId: 'tool-1',
        state: 'pending',
        createdAt: 11,
      },
      {
        sessionId: 'session-123',
        requestId: 'req-0',
        title: 'Delete tmp file',
        toolCallId: 'tool-0',
        state: 'resolved',
        outcome: 'approved',
        createdAt: 8,
        resolvedAt: 9,
      },
    ],
    chunks: [
      {
        sessionId: 'session-123',
        requestId: 'req-1',
        toolCallId: 'tool-1',
        createdAt: 12,
        update: {
          sessionUpdate: 'tool_call',
          toolCallId: 'tool-1',
          title: 'Edit src/index.ts',
          status: 'pending',
        },
      },
      {
        sessionId: 'session-123',
        requestId: 'req-1',
        createdAt: 13,
        update: {
          sessionUpdate: 'agent_message_chunk',
          content: {
            type: 'text',
            text: 'I am looking at the failing test output now.',
          },
        },
      },
    ],
  })

  const output = renderToString(
    React.createElement(SessionStatePane, {
      acpUrl: 'ws://127.0.0.1:55371/acp',
      db,
      runtimeId: 'runtime:46ae8df5-5588-482c-a1ea-c85b1b49723d',
      serverUrl: 'http://127.0.0.1:4440',
      sessionId: 'session-123',
      stateStreamUrl:
        'http://127.0.0.1:7474/v1/stream/fireline-state-runtime-46ae8df5-5588-482c-a1ea-c85b1b49723d',
    }),
    { columns: 100 },
  )

  assert.match(output, /Session state/)
  assert.match(output, /runtime runtime:46ae8df5/)
  assert.match(output, /Prompt turn/)
  assert.match(output, /Investigate the failing build/)
  assert.match(output, /chunk summary/)
  assert.match(output, /Approval · pending/i)
  assert.match(output, /Approval · allowed/i)
})

function createFakeDb(seed: {
  readonly chunks: readonly ChunkRow[]
  readonly permissions: readonly PermissionRow[]
  readonly promptRequests: readonly PromptRequestRow[]
  readonly sessions: readonly SessionRow[]
}): FirelineDB {
  return {
    sessions: createFakeCollection(seed.sessions),
    promptRequests: createFakeCollection(seed.promptRequests),
    permissions: createFakeCollection(seed.permissions),
    chunks: createFakeCollection(seed.chunks),
    collections: {
      sessions: createFakeCollection(seed.sessions),
      promptRequests: createFakeCollection(seed.promptRequests),
      permissions: createFakeCollection(seed.permissions),
      chunks: createFakeCollection(seed.chunks),
    },
    close() {},
    preload: async () => {},
    stream: {} as FirelineDB['stream'],
    utils: {} as FirelineDB['utils'],
  } as unknown as FirelineDB
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
