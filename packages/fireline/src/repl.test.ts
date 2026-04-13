import assert from 'node:assert/strict'
import test from 'node:test'
import { renderToString } from 'ink'
import React from 'react'
import { FirelineReplApp, partitionTranscriptEntries } from './repl-ui.js'
import { ReplController } from './repl.js'

test('repl controller submits prompts and renders streamed output', async () => {
  const prompts: string[] = []

  const controller = new ReplController({
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

  controller.setPendingApproval({
    requestId: 'request-123',
    sessionId: 'session-123',
    summary: 'Write test.txt to /workspace',
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

  assert.match(output, /approval pending/i)
  assert.match(output, /Write test\.txt to \/workspace/)
  assert.match(output, /Press y to allow or n to deny\./)

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
    busy: true,
    entries: transcript,
    pendingApproval: null,
    pendingTools: 1,
    resolvingApproval: false,
    serverUrl: 'http://127.0.0.1:4440',
    sessionId: 'session-123',
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
    busy: false,
    entries: transcript,
    pendingApproval: null,
    pendingTools: 0,
    resolvingApproval: false,
    serverUrl: 'http://127.0.0.1:4440',
    sessionId: 'session-123',
    usage: null,
  })

  assert.deepEqual(
    idle.committedEntries.map((entry) => entry.id),
    [1, 2, 3, 4, 5],
  )
  assert.deepEqual(idle.liveEntries, [])
})
