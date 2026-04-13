import { describe, expect, it, vi } from 'vitest'
import { createCollection, localOnlyCollectionOptions } from '@tanstack/db'

const FORBIDDEN_FIELDS = ['hostKey', 'runtimeKey', 'hostId', 'runtimeId', 'nodeId'] as const

function makeCollection<T extends object>(rows: T[], getKey: (row: T) => string) {
  const collection = createCollection(
    localOnlyCollectionOptions<T, string>({
      getKey,
    }),
  )
  for (const row of rows) {
    collection.insert(row)
  }
  return collection
}

describe('fireline.db plane separation', () => {
  it('exposes only agent-plane collections and rows', async () => {
    const actualState = await vi.importActual<typeof import('@fireline/state')>('@fireline/state')

    expect(Object.keys(actualState.firelineState).sort()).toEqual([
      'chunks',
      'connections',
      'permissions',
      'promptRequests',
      'sessions',
      'terminals',
    ])

    vi.resetModules()
    vi.doMock('@fireline/state', async () => ({
      ...actualState,
      createFirelineDB: () => ({
        collections: {
          connections: makeCollection(
            [{ logicalConnectionId: 'conn-1', state: 'attached', createdAt: 1, updatedAt: 1 }],
            (row) => row.logicalConnectionId,
          ),
          promptRequests: makeCollection(
            [{ sessionId: 'sess-1', requestId: 'req-1', state: 'active', startedAt: 1 }],
            (row) => `${row.sessionId}:${String(row.requestId)}`,
          ),
          permissions: makeCollection(
            [{ sessionId: 'sess-1', requestId: 'req-1', state: 'pending', createdAt: 1 }],
            (row) => `${row.sessionId}:${String(row.requestId)}`,
          ),
          terminals: makeCollection(
            [
              {
                terminalId: 'term-1',
                logicalConnectionId: 'conn-1',
                sessionId: 'sess-1',
                state: 'open',
                createdAt: 1,
                updatedAt: 1,
              },
            ],
            (row) => row.terminalId,
          ),
          sessions: makeCollection(
            [
              {
                sessionId: 'sess-1',
                state: 'active',
                supportsLoadSession: true,
                createdAt: 1,
                updatedAt: 1,
                lastSeenAt: 1,
              },
            ],
            (row) => row.sessionId,
          ),
          chunks: makeCollection(
            [
              {
                sessionId: 'sess-1',
                requestId: 'req-1',
                update: {
                  sessionUpdate: 'agent_message_chunk',
                  content: { type: 'text', text: 'hello' },
                },
                createdAt: 1,
              },
            ],
            (row) => `${row.sessionId}:${String(row.requestId)}:${row.createdAt}`,
          ),
        },
        stream: {},
        utils: {},
        async preload() {},
        close() {},
      }),
    }))

    const { db } = await import('../src/db.js')
    const firelineDb = await db({ stateStreamUrl: 'http://example.test/v1/stream/state' })

    expect('runtimeInstances' in firelineDb).toBe(false)
    expect(Object.keys(firelineDb.collections).sort()).toEqual([
      'chunks',
      'connections',
      'permissions',
      'promptRequests',
      'sessions',
      'terminals',
    ])

    for (const collection of Object.values(firelineDb.collections)) {
      for (const row of collection.toArray as Array<Record<string, unknown>>) {
        for (const field of FORBIDDEN_FIELDS) {
          expect(row).not.toHaveProperty(field)
        }
      }
    }
  })
})
