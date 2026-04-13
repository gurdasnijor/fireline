import assert from 'node:assert/strict'
import test from 'node:test'
import { renderToString } from 'ink'
import React from 'react'
import {
  createAcpEventAdapter,
  createControlEventBus,
  EventStreamPane,
  EventStreamStore,
  filterEventStreamEvents,
  mapDurableBatchToEvents,
  type RealtimeEvent,
} from './repl-pane-events.js'

function event(overrides: Partial<RealtimeEvent> = {}): RealtimeEvent {
  return {
    id: overrides.id ?? `event-${Math.random()}`,
    name: overrides.name ?? 'session/prompt',
    payload: overrides.payload ?? 'payload',
    severity: overrides.severity,
    sessionId: overrides.sessionId ?? 'session-1',
    source: overrides.source ?? 'acp',
    requestId: overrides.requestId ?? null,
    timestamp: overrides.timestamp ?? 1_710_000_000_000,
  }
}

test('event stream store sorts chronologically and keeps the newest rows inside the cap', () => {
  const store = new EventStreamStore(2)

  store.append(event({ id: 'late', name: 'late', timestamp: 300 }))
  store.append(event({ id: 'early', name: 'early', timestamp: 100 }))
  store.append(event({ id: 'middle', name: 'middle', timestamp: 200 }))

  assert.deepEqual(
    store.getSnapshot().events.map((row) => row.name),
    ['middle', 'late'],
  )
})

test('filterEventStreamEvents applies source prefixes and free-text matching', () => {
  const events = [
    event({ id: 'a', source: 'acp', name: 'session/prompt', payload: 'prompt=hello docker smoke' }),
    event({ id: 'd', source: 'durable', name: 'permissions.append', payload: 'state=pending' }),
    event({ id: 'c', source: 'control', name: 'sandbox.provisioned', payload: 'runtime booted' }),
  ]

  assert.deepEqual(
    filterEventStreamEvents(events, 'acp:prompt').map((row) => row.id),
    ['a'],
  )
  assert.deepEqual(
    filterEventStreamEvents(events, 'durable:pending').map((row) => row.id),
    ['d'],
  )
  assert.deepEqual(
    filterEventStreamEvents(events, 'booted').map((row) => row.id),
    ['c'],
  )
})

test('ACP adapter records session lifecycle requests and notifications with canonical names', async () => {
  const rows: RealtimeEvent[] = []
  const adapter = createAcpEventAdapter({
    connection: {
      cancel: async () => undefined,
      loadSession: async () => ({ ok: true }),
      newSession: async () => ({ sessionId: 'session-1' }),
      prompt: async () => ({ ok: true }),
      unstable_resumeSession: async () => ({ ok: true }),
    },
    sink: {
      append(row) {
        rows.push({
          id: 'id' in row ? row.id : `${row.source}:${rows.length}`,
          ...row,
        })
      },
      noteControlSurfaceGap() {},
    },
  })

  await adapter.connection.newSession({ cwd: '/workspace' } as any)
  await adapter.connection.prompt({
    prompt: [{ type: 'text', text: 'hello from pane 3' }],
    sessionId: 'session-1',
  } as any)
  await adapter.connection.loadSession({ sessionId: 'session-1' } as any)
  await adapter.connection.cancel?.({ requestId: 'req-9', sessionId: 'session-1' } as any)
  adapter.recordNotification({
    sessionId: 'session-1',
    update: {
      content: { text: 'streaming chunk', type: 'text' },
      requestId: 'req-9',
      sessionUpdate: 'agent_message_chunk',
    },
  } as any)

  assert.deepEqual(
    rows.map((row) => row.name),
    ['session/new', 'session/prompt', 'session/load', 'session/cancel', 'session_update'],
  )
  assert.match(rows[0]!.payload, /dir=out/)
  assert.match(rows[1]!.payload, /prompt=hello from pane 3/)
  assert.match(rows[4]!.payload, /dir=in/)
  assert.match(rows[4]!.payload, /kind=agent_message_chunk/)
})

test('durable batch mapping normalizes collection labels and filters by session id', () => {
  const batch = {
    items: [
      {
        headers: { operation: 'append' },
        key: 'prompt-1',
        type: 'prompt_request',
        value: {
          requestId: 'req-1',
          sessionId: 'session-1',
          startedAt: 100,
          state: 'active',
        },
      },
      {
        key: 'chunk-1',
        type: 'chunk_v2',
        value: {
          createdAt: 101,
          requestId: 'req-1',
          sessionId: 'session-1',
          update: { sessionUpdate: 'agent_message_chunk' },
        },
      },
      {
        key: 'permission-2',
        type: 'permission',
        value: {
          createdAtMs: 102,
          kind: 'permission_request',
          requestId: 'req-2',
          sessionId: 'session-2',
          state: 'pending',
        },
      },
      {
        key: 'ignored-1',
        type: 'runtime_instance',
        value: { createdAt: 103 },
      },
    ],
    streamClosed: false,
    upToDate: true,
  } as unknown as Parameters<typeof mapDurableBatchToEvents>[0]

  assert.deepEqual(
    mapDurableBatchToEvents(batch, 'session-1').map((row) => row.name),
    ['promptTurns.append', 'turnChunks.append'],
  )
})

test('control bus emits rows and the pane renders the note and event list', () => {
  const store = new EventStreamStore()
  const control = createControlEventBus(store)

  control.noteMissingSurface()
  control.emit({
    name: 'host.boot',
    payload: 'operator lifecycle event',
    severity: 'info',
    sessionId: 'session-1',
    timestamp: 1_710_000_000_123,
  })
  store.append(event({
    id: 'acp-1',
    name: 'session/prompt',
    payload: 'dir=out prompt=hello',
    timestamp: 1_710_000_000_124,
  }))

  const output = renderToString(
    React.createElement(EventStreamPane, {
      controller: store,
      focused: false,
      maxVisibleRows: 5,
    }),
    { columns: 100 },
  )

  assert.match(output, /Realtime events/)
  assert.match(output, /control note:/)
  assert.match(output, /\[control\]/)
  assert.match(output, /host\.boot/)
  assert.match(output, /\[acp\]/)
  assert.match(output, /session\/prompt/)
})
