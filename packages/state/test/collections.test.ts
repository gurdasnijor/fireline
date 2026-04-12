import { describe, expect, it } from 'vitest'
import { createCollection, localOnlyCollectionOptions } from '@tanstack/db'

import type { ChunkRow, PermissionRow, PromptTurnRow } from '../src/schema.js'
import {
  createConnectionTurnsCollection,
  extractChunkTextPreview,
  requestIdCollectionKey,
  createSessionPermissionsCollection,
  createSessionTurnsCollection,
  createTurnChunksCollection,
} from '../src/index.js'

function makePromptTurnsCollection() {
  return createCollection(
    localOnlyCollectionOptions<PromptTurnRow>({
      getKey: (row) => row.promptTurnId,
    }),
  )
}

function makeChunksCollection() {
  return createCollection(
    localOnlyCollectionOptions<ChunkRow>({
      getKey: (row) => row.chunkKey,
    }),
  )
}

function makePermissionsCollection() {
  return createCollection(
    localOnlyCollectionOptions<PermissionRow>({
      getKey: (row) => requestIdCollectionKey(row.requestId),
    }),
  )
}

function promptTurn(overrides: Partial<PromptTurnRow> & Pick<PromptTurnRow, 'promptTurnId'>): PromptTurnRow {
  return {
    promptTurnId: overrides.promptTurnId,
    logicalConnectionId: overrides.logicalConnectionId ?? 'conn-1',
    sessionId: overrides.sessionId ?? 'session-1',
    requestId: overrides.requestId ?? `req-${overrides.promptTurnId}`,
    text: overrides.text,
    state: overrides.state ?? 'active',
    startedAt: overrides.startedAt ?? 0,
    completedAt: overrides.completedAt,
    stopReason: overrides.stopReason,
    position: overrides.position,
    traceId: overrides.traceId,
    parentPromptTurnId: overrides.parentPromptTurnId,
  }
}

function chunk(overrides: Partial<ChunkRow> & Pick<ChunkRow, 'chunkKey'>): ChunkRow {
  return {
    chunkKey: overrides.chunkKey,
    sessionId: overrides.sessionId ?? 'session-1',
    requestId: overrides.requestId ?? 'req-1',
    toolCallId: overrides.toolCallId,
    update:
      overrides.update ??
      ({
        sessionUpdate: 'agent_message_chunk',
        content: { type: 'text', text: '' },
      } as ChunkRow['update']),
    createdAt: overrides.createdAt ?? 0,
  }
}

function permission(
  overrides: Partial<PermissionRow> & Pick<PermissionRow, 'requestId'>,
): PermissionRow {
  return {
    requestId: overrides.requestId,
    jsonrpcId: overrides.jsonrpcId ?? overrides.requestId,
    logicalConnectionId: overrides.logicalConnectionId ?? 'conn-1',
    sessionId: overrides.sessionId ?? 'session-1',
    promptTurnId: overrides.promptTurnId ?? 'turn-1',
    title: overrides.title,
    toolCallId: overrides.toolCallId,
    options: overrides.options,
    state: overrides.state ?? 'pending',
    outcome: overrides.outcome,
    createdAt: overrides.createdAt ?? 0,
    resolvedAt: overrides.resolvedAt,
  }
}

describe('createSessionTurnsCollection', () => {
  it('returns only turns for the target session, ordered by startedAt', async () => {
    const promptTurns = makePromptTurnsCollection()
    promptTurns.insert(promptTurn({ promptTurnId: 'a', sessionId: 'session-1', startedAt: 200 }))
    promptTurns.insert(promptTurn({ promptTurnId: 'b', sessionId: 'session-2', startedAt: 150 }))
    promptTurns.insert(promptTurn({ promptTurnId: 'c', sessionId: 'session-1', startedAt: 100 }))

    const sessionTurns = createSessionTurnsCollection({ promptTurns, sessionId: 'session-1' })
    await sessionTurns.preload()

    expect(sessionTurns.toArray.map((row) => row.promptTurnId)).toEqual(['c', 'a'])
  })

  it('excludes turns from other sessions even as rows arrive', async () => {
    const promptTurns = makePromptTurnsCollection()
    const sessionTurns = createSessionTurnsCollection({ promptTurns, sessionId: 'session-1' })
    await sessionTurns.preload()

    promptTurns.insert(promptTurn({ promptTurnId: 'a', sessionId: 'session-2', startedAt: 100 }))
    promptTurns.insert(promptTurn({ promptTurnId: 'b', sessionId: 'session-1', startedAt: 200 }))

    expect(sessionTurns.toArray.map((row) => row.promptTurnId)).toEqual(['b'])
  })
})

describe('createConnectionTurnsCollection', () => {
  it('returns only turns for the target logical connection, ordered by startedAt', async () => {
    const promptTurns = makePromptTurnsCollection()
    promptTurns.insert(
      promptTurn({ promptTurnId: 'a', logicalConnectionId: 'conn-1', startedAt: 50 }),
    )
    promptTurns.insert(
      promptTurn({ promptTurnId: 'b', logicalConnectionId: 'conn-2', startedAt: 60 }),
    )
    promptTurns.insert(
      promptTurn({
        promptTurnId: 'c',
        logicalConnectionId: 'conn-1',
        sessionId: 'session-2',
        startedAt: 10,
      }),
    )

    const connectionTurns = createConnectionTurnsCollection({
      promptTurns,
      logicalConnectionId: 'conn-1',
    })
    await connectionTurns.preload()

    expect(connectionTurns.toArray.map((row) => row.promptTurnId)).toEqual(['c', 'a'])
  })
})

describe('createTurnChunksCollection', () => {
  it('returns only chunks for the target request, ordered by createdAt', async () => {
    const chunks = makeChunksCollection()
    chunks.insert(
      chunk({
        chunkKey: 'chunk-1',
        requestId: 'req-1',
        createdAt: 20,
        update: {
          sessionUpdate: 'agent_message_chunk',
          content: { type: 'text', text: 'world' },
        } as ChunkRow['update'],
      }),
    )
    chunks.insert(
      chunk({
        chunkKey: 'chunk-2',
        requestId: 'req-1',
        createdAt: 10,
        update: {
          sessionUpdate: 'agent_message_chunk',
          content: { type: 'text', text: 'hello ' },
        } as ChunkRow['update'],
      }),
    )
    chunks.insert(
      chunk({
        chunkKey: 'chunk-3',
        requestId: 'req-2',
        createdAt: 5,
        update: {
          sessionUpdate: 'agent_message_chunk',
          content: { type: 'text', text: 'other' },
        } as ChunkRow['update'],
      }),
    )

    const turnChunks = createTurnChunksCollection({ chunks, requestId: 'req-1' })
    await turnChunks.preload()

    expect(turnChunks.toArray.map((row) => row.chunkKey)).toEqual(['chunk-2', 'chunk-1'])
    expect(turnChunks.toArray.map((row) => extractChunkTextPreview(row.update)).join('')).toBe(
      'hello world',
    )
  })
})

describe('createSessionPermissionsCollection', () => {
  it('returns all permissions for the session regardless of state, ordered by createdAt', async () => {
    const permissions = makePermissionsCollection()
    permissions.insert(
      permission({ requestId: 'p-1', sessionId: 'session-1', state: 'pending', createdAt: 200 }),
    )
    permissions.insert(
      permission({
        requestId: 'p-2',
        sessionId: 'session-1',
        state: 'resolved',
        outcome: 'cancelled',
        createdAt: 100,
      }),
    )
    permissions.insert(
      permission({ requestId: 'p-3', sessionId: 'session-2', state: 'pending', createdAt: 150 }),
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
