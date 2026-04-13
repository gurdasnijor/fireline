import { describe, expect, it } from 'vitest'
import { createCollection, localOnlyCollectionOptions } from '@tanstack/db'

import type { ChunkRow, PermissionRow, PromptRequestRow } from '../src/schema.js'
import {
  createRequestChunksCollection,
  createSessionPermissionsCollection,
  createSessionPromptRequestsCollection,
  extractChunkTextPreview,
  promptRequestCollectionKey,
} from '../src/index.js'

function makePromptRequestsCollection() {
  return createCollection(
    localOnlyCollectionOptions<PromptRequestRow>({
      getKey: (row) => promptRequestCollectionKey(row.sessionId, row.requestId),
    }),
  )
}

function makeChunksCollection() {
  return createCollection(
    localOnlyCollectionOptions<ChunkRow>({
      getKey: (row) => `${row.sessionId}:${row.requestId}:${row.toolCallId ?? 'none'}:${row.createdAt}`,
    }),
  )
}

function makePermissionsCollection() {
  return createCollection(
    localOnlyCollectionOptions<PermissionRow>({
      getKey: (row) => promptRequestCollectionKey(row.sessionId, row.requestId),
    }),
  )
}

function promptRequest(
  overrides: Partial<PromptRequestRow> & Pick<PromptRequestRow, 'requestId'>,
): PromptRequestRow {
  return {
    sessionId: overrides.sessionId ?? 'session-1',
    requestId: overrides.requestId,
    text: overrides.text,
    state: overrides.state ?? 'active',
    position: overrides.position,
    startedAt: overrides.startedAt ?? 0,
    completedAt: overrides.completedAt,
    stopReason: overrides.stopReason,
  }
}

function chunk(overrides: Partial<ChunkRow> & Pick<ChunkRow, 'createdAt'>): ChunkRow {
  return {
    sessionId: overrides.sessionId ?? 'session-1',
    requestId: overrides.requestId ?? 'req-1',
    toolCallId: overrides.toolCallId,
    update:
      overrides.update ??
      ({
        sessionUpdate: 'agent_message_chunk',
        content: { type: 'text', text: '' },
      } as ChunkRow['update']),
    createdAt: overrides.createdAt,
  }
}

function permission(
  overrides: Partial<PermissionRow> &
    Pick<PermissionRow, 'sessionId' | 'requestId'>,
): PermissionRow {
  return {
    sessionId: overrides.sessionId,
    requestId: overrides.requestId,
    title: overrides.title,
    toolCallId: overrides.toolCallId,
    options: overrides.options,
    state: overrides.state ?? 'pending',
    outcome: overrides.outcome,
    createdAt: overrides.createdAt ?? 0,
    resolvedAt: overrides.resolvedAt,
  }
}

describe('createSessionPromptRequestsCollection', () => {
  it('returns only prompt requests for the target session, ordered by startedAt', async () => {
    const promptRequests = makePromptRequestsCollection()
    promptRequests.insert(promptRequest({ requestId: 'a', sessionId: 'session-1', startedAt: 200 }))
    promptRequests.insert(promptRequest({ requestId: 'b', sessionId: 'session-2', startedAt: 150 }))
    promptRequests.insert(promptRequest({ requestId: 'c', sessionId: 'session-1', startedAt: 100 }))

    const sessionRequests = createSessionPromptRequestsCollection({
      promptRequests,
      sessionId: 'session-1',
    })
    await sessionRequests.preload()

    expect(sessionRequests.toArray.map((row) => row.requestId)).toEqual(['c', 'a'])
  })

  it('excludes prompt requests from other sessions even as rows arrive', async () => {
    const promptRequests = makePromptRequestsCollection()
    const sessionRequests = createSessionPromptRequestsCollection({
      promptRequests,
      sessionId: 'session-1',
    })
    await sessionRequests.preload()

    promptRequests.insert(promptRequest({ requestId: 'a', sessionId: 'session-2', startedAt: 100 }))
    promptRequests.insert(promptRequest({ requestId: 'b', sessionId: 'session-1', startedAt: 200 }))

    expect(sessionRequests.toArray.map((row) => row.requestId)).toEqual(['b'])
  })
})

describe('createRequestChunksCollection', () => {
  it('returns only chunks for the target request, ordered by createdAt', async () => {
    const chunks = makeChunksCollection()
    chunks.insert(
      chunk({
        createdAt: 20,
        requestId: 'req-1',
        update: {
          sessionUpdate: 'agent_message_chunk',
          content: { type: 'text', text: 'world' },
        } as ChunkRow['update'],
      }),
    )
    chunks.insert(
      chunk({
        createdAt: 10,
        requestId: 'req-1',
        update: {
          sessionUpdate: 'agent_message_chunk',
          content: { type: 'text', text: 'hello ' },
        } as ChunkRow['update'],
      }),
    )
    chunks.insert(
      chunk({
        createdAt: 5,
        requestId: 'req-2',
        update: {
          sessionUpdate: 'agent_message_chunk',
          content: { type: 'text', text: 'other' },
        } as ChunkRow['update'],
      }),
    )

    const requestChunks = createRequestChunksCollection({
      chunks,
      sessionId: 'session-1',
      requestId: 'req-1',
    })
    await requestChunks.preload()

    expect(requestChunks.toArray.map((row) => row.createdAt)).toEqual([10, 20])
    expect(requestChunks.toArray.map((row) => extractChunkTextPreview(row.update)).join('')).toBe(
      'hello world',
    )
  })
})

describe('createSessionPermissionsCollection', () => {
  it('returns all permissions for the session regardless of state, ordered by createdAt', async () => {
    const permissions = makePermissionsCollection()
    permissions.insert(
      permission({ sessionId: 'session-1', requestId: 'p-1', state: 'pending', createdAt: 200 }),
    )
    permissions.insert(
      permission({
        sessionId: 'session-1',
        requestId: 'p-2',
        state: 'resolved',
        outcome: 'approved',
        createdAt: 100,
      }),
    )
    permissions.insert(
      permission({ sessionId: 'session-2', requestId: 'p-3', state: 'pending', createdAt: 150 }),
    )

    const sessionPermissions = createSessionPermissionsCollection({
      permissions,
      sessionId: 'session-1',
    })
    await sessionPermissions.preload()

    expect(sessionPermissions.toArray.map((row) => row.requestId)).toEqual(['p-2', 'p-1'])
    expect(sessionPermissions.toArray.map((row) => row.state)).toEqual(['resolved', 'pending'])
  })
})
