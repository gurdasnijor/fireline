import { createStreamDB, type StreamDB, type StreamDBMethods } from '@durable-streams/state'
import { createCollection, localOnlyCollectionOptions, type Collection } from '@tanstack/db'

import { promptRequestCollectionKey } from './acp-types.js'
import { firelineStreamState } from './schema.js'
import type {
  ChunkEventRow,
  ChunkRow,
  ConnectionRow,
  LegacyChunkEventRow,
  LegacyPromptTurnEventRow,
  LegacySessionEventRow,
  PermissionEventRow,
  PermissionRow,
  PromptRequestEventRow,
  PromptRequestRow,
  RuntimeInstanceRow,
  SessionEventRow,
  SessionRow,
  TerminalRow,
} from './schema.js'
import {
  chunkRowKey,
  permissionRowKey,
  promptRequestRowKey,
  withCollectionKey,
} from './schema.js'

export type ObservableCollection<T extends object> = Collection<T, string> & {
  subscribe(callback: (rows: T[]) => void): { unsubscribe(): void }
}

export interface FirelineCollections {
  connections: ObservableCollection<ConnectionRow>
  promptRequests: ObservableCollection<PromptRequestRow>
  permissions: ObservableCollection<PermissionRow>
  terminals: ObservableCollection<TerminalRow>
  runtimeInstances: ObservableCollection<RuntimeInstanceRow>
  sessions: ObservableCollection<SessionRow>
  chunks: ObservableCollection<ChunkRow>
}

export type FirelineDB = {
  collections: FirelineCollections
} & StreamDBMethods

type RawFirelineDB = StreamDB<typeof firelineStreamState>

export interface FirelineDBConfig {
  stateStreamUrl: string
  headers?: Record<string, string>
  signal?: AbortSignal
}

export function createFirelineDB(config: FirelineDBConfig): FirelineDB {
  const { stateStreamUrl, headers, signal } = config

  const rawDb: RawFirelineDB = createStreamDB({
    streamOptions: {
      url: stateStreamUrl,
      contentType: 'application/json',
      headers,
      signal,
    },
    state: firelineStreamState,
  })

  const collections: FirelineCollections = {
    connections: createObservableLocalCollection((row) => row.logicalConnectionId),
    promptRequests: createObservableLocalCollection((row) => promptRequestRowKey(row)),
    permissions: createObservableLocalCollection((row) => permissionRowKey(row)),
    terminals: createObservableLocalCollection((row) => row.terminalId),
    runtimeInstances: createObservableLocalCollection((row) => row.instanceId),
    sessions: createObservableLocalCollection((row) => row.sessionId),
    chunks: createObservableLocalCollection((row) => chunkRowKey(row)),
  }

  const syncAll = () => {
    reconcileCollection(
      collections.connections,
      rawDb.collections.connections.toArray.map(cloneConnectionRow),
    )
    reconcileCollection(
      collections.promptRequests,
      projectPromptRequests(
        rawDb.collections.promptRequests.toArray as PromptRequestEventRow[],
        rawDb.collections.legacyPromptTurns.toArray as LegacyPromptTurnEventRow[],
      ),
    )
    reconcileCollection(
      collections.permissions,
      projectPermissions(rawDb.collections.permissions.toArray as PermissionEventRow[]),
    )
    reconcileCollection(
      collections.terminals,
      rawDb.collections.terminals.toArray.map(cloneTerminalRow),
    )
    reconcileCollection(
      collections.runtimeInstances,
      rawDb.collections.runtimeInstances.toArray.map(cloneRuntimeInstanceRow),
    )
    reconcileCollection(
      collections.sessions,
      projectSessions(
        rawDb.collections.sessions.toArray as SessionEventRow[],
        rawDb.collections.legacySessions.toArray as LegacySessionEventRow[],
      ),
    )
    reconcileCollection(
      collections.chunks,
      projectChunks(
        rawDb.collections.chunks.toArray as ChunkEventRow[],
        rawDb.collections.legacyChunks.toArray as LegacyChunkEventRow[],
        rawDb.collections.promptRequests.toArray as PromptRequestEventRow[],
        rawDb.collections.legacyPromptTurns.toArray as LegacyPromptTurnEventRow[],
      ),
    )
  }

  const unsubscribers = [
    rawDb.collections.connections.subscribeChanges(syncAll),
    rawDb.collections.promptRequests.subscribeChanges(syncAll),
    rawDb.collections.permissions.subscribeChanges(syncAll),
    rawDb.collections.terminals.subscribeChanges(syncAll),
    rawDb.collections.runtimeInstances.subscribeChanges(syncAll),
    rawDb.collections.sessions.subscribeChanges(syncAll),
    rawDb.collections.chunks.subscribeChanges(syncAll),
    rawDb.collections.legacyPromptTurns.subscribeChanges(syncAll),
    rawDb.collections.legacySessions.subscribeChanges(syncAll),
    rawDb.collections.legacyChunks.subscribeChanges(syncAll),
  ]

  return {
    collections,
    stream: rawDb.stream,
    utils: rawDb.utils,
    async preload() {
      await rawDb.preload()
      syncAll()
    },
    close() {
      for (const subscription of unsubscribers) {
        subscription.unsubscribe()
      }
      rawDb.close()
    },
  }
}

function createObservableLocalCollection<T extends object>(
  getKey: (row: T) => string,
): ObservableCollection<T> {
  const collection = createCollection(
    localOnlyCollectionOptions<T, string>({
      getKey,
    }),
  ) as unknown as ObservableCollection<T>
  attachSubscribe(collection)
  return collection
}

function reconcileCollection<T extends object>(
  collection: ObservableCollection<T>,
  rows: T[],
): void {
  const desired = new Map<string, T>()
  for (const row of rows) {
    desired.set(collection.getKeyFromItem(row), row)
  }

  for (const key of Array.from(collection.keys())) {
    if (!desired.has(key)) {
      collection.delete(key)
    }
  }

  for (const [key, row] of desired) {
    const current = collection.get(key)
    if (!current) {
      collection.insert(row)
      continue
    }

    if (JSON.stringify(current) !== JSON.stringify(row)) {
      collection.delete(key)
      collection.insert(row)
    }
  }
}

function cloneConnectionRow(row: ConnectionRow): ConnectionRow {
  return {
    logicalConnectionId: row.logicalConnectionId,
    state: row.state,
    latestSessionId: row.latestSessionId,
    lastError: row.lastError,
    queuePaused: row.queuePaused,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }
}

function cloneTerminalRow(row: TerminalRow): TerminalRow {
  return {
    terminalId: row.terminalId,
    logicalConnectionId: row.logicalConnectionId,
    sessionId: row.sessionId,
    promptTurnId: row.promptTurnId,
    state: row.state,
    command: row.command,
    exitCode: row.exitCode,
    signal: row.signal,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }
}

function cloneRuntimeInstanceRow(row: RuntimeInstanceRow): RuntimeInstanceRow {
  return {
    instanceId: row.instanceId,
    runtimeName: row.runtimeName,
    status: row.status,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }
}

function projectPromptRequests(
  canonicalRows: PromptRequestEventRow[],
  legacyRows: LegacyPromptTurnEventRow[],
): PromptRequestRow[] {
  const projected = new Map<string, PromptRequestRow>()

  for (const row of legacyRows) {
    const key = promptRequestCollectionKey(row.sessionId, row.requestId)
    projected.set(
      key,
      withCollectionKey(
        {
          sessionId: row.sessionId,
          requestId: row.requestId,
          text: row.text,
          state: row.state,
          position: row.position,
          stopReason: row.stopReason,
          startedAt: row.startedAt,
          completedAt: row.completedAt,
        },
        key,
      ),
    )
  }

  for (const row of canonicalRows) {
    const key = promptRequestCollectionKey(row.sessionId, row.requestId)
    projected.set(
      key,
      withCollectionKey(
        {
          sessionId: row.sessionId,
          requestId: row.requestId,
          text: row.text,
          state: row.state,
          position: row.position,
          stopReason: row.stopReason,
          startedAt: row.startedAt,
          completedAt: row.completedAt,
        },
        key,
      ),
    )
  }

  return [...projected.values()]
}

function projectPermissions(events: PermissionEventRow[]): PermissionRow[] {
  const grouped = new Map<
    string,
    {
      request?: PermissionEventRow
      resolution?: PermissionEventRow
    }
  >()

  for (const event of events) {
    if (event.requestId === undefined) {
      continue
    }

    const key = promptRequestCollectionKey(event.sessionId, event.requestId)
    const current = grouped.get(key) ?? {}

    if (event.kind === 'permission_request') {
      if (!current.request || event.createdAtMs < current.request.createdAtMs) {
        current.request = event
      }
    } else if (!current.resolution || event.createdAtMs >= current.resolution.createdAtMs) {
      current.resolution = event
    }

    grouped.set(key, current)
  }

  return [...grouped.entries()].map(([key, entry]) => {
    const seed = entry.request ?? entry.resolution
    if (!seed || seed.requestId === undefined) {
      throw new Error(`permission projection missing canonical request id for key ${key}`)
    }

    return withCollectionKey(
      {
        sessionId: seed.sessionId,
        requestId: seed.requestId,
        title: entry.request?.reason,
        toolCallId: entry.request?.toolCallId ?? entry.resolution?.toolCallId,
        state: entry.request
          ? entry.resolution
            ? 'resolved'
            : 'pending'
          : 'orphaned',
        outcome: entry.resolution
          ? entry.resolution.allow === true
            ? 'approved'
            : 'denied'
          : undefined,
        createdAt: entry.request?.createdAtMs ?? seed.createdAtMs,
        resolvedAt: entry.resolution?.createdAtMs,
      },
      key,
    )
  })
}

function projectSessions(
  canonicalRows: SessionEventRow[],
  legacyRows: LegacySessionEventRow[],
): SessionRow[] {
  const projected = new Map<string, SessionRow>()

  for (const row of legacyRows) {
    projected.set(row.sessionId, {
      sessionId: row.sessionId,
      state: row.state,
      supportsLoadSession: row.supportsLoadSession,
      createdAt: row.createdAt,
      updatedAt: row.updatedAt,
      lastSeenAt: row.lastSeenAt,
    })
  }

  for (const row of canonicalRows) {
    projected.set(row.sessionId, {
      sessionId: row.sessionId,
      state: row.state,
      supportsLoadSession: row.supportsLoadSession,
      createdAt: row.createdAt,
      updatedAt: row.updatedAt,
      lastSeenAt: row.lastSeenAt,
    })
  }

  return [...projected.values()]
}

function projectChunks(
  canonicalRows: ChunkEventRow[],
  legacyRows: LegacyChunkEventRow[],
  canonicalPrompts: PromptRequestEventRow[],
  legacyPrompts: LegacyPromptTurnEventRow[],
): ChunkRow[] {
  const projected = new Map<string, ChunkRow>()
  const promptRefsByLegacyKey = new Map<
    string,
    {
      sessionId: ChunkRow['sessionId']
      requestId: ChunkRow['requestId']
    }
  >()
  const canonicalPromptKeysWithChunks = new Set<string>()

  for (const row of legacyPrompts) {
    promptRefsByLegacyKey.set(row.promptTurnId, {
      sessionId: row.sessionId,
      requestId: row.requestId,
    })
  }

  for (const row of canonicalPrompts) {
    promptRefsByLegacyKey.set(row.promptRequestKey, {
      sessionId: row.sessionId,
      requestId: row.requestId,
    })
  }

  for (const row of canonicalRows) {
    const promptKey = promptRequestCollectionKey(row.sessionId, row.requestId)
    canonicalPromptKeysWithChunks.add(promptKey)
    projected.set(
      row.chunkKey,
      withCollectionKey(
        {
          sessionId: row.sessionId,
          requestId: row.requestId,
          toolCallId: row.toolCallId,
          update: row.update,
          createdAt: row.createdAt,
        },
        row.chunkKey,
      ),
    )
  }

  for (const row of legacyRows) {
    const promptRef = promptRefsByLegacyKey.get(row.promptTurnId)
    if (!promptRef) {
      continue
    }

    const promptKey = promptRequestCollectionKey(promptRef.sessionId, promptRef.requestId)
    if (canonicalPromptKeysWithChunks.has(promptKey)) {
      continue
    }

    projected.set(
      row.chunkId,
      withCollectionKey(
        {
          sessionId: promptRef.sessionId,
          requestId: promptRef.requestId,
          toolCallId: undefined,
          update: legacyChunkToSessionUpdate(row),
          createdAt: row.createdAt,
        },
        row.chunkId,
      ),
    )
  }

  return [...projected.values()]
}

function legacyChunkToSessionUpdate(row: LegacyChunkEventRow): ChunkRow['update'] {
  const text = row.content

  switch (row.type) {
    case 'thinking':
      return {
        sessionUpdate: 'agent_thought_chunk',
        content: { type: 'text', text },
      } as ChunkRow['update']

    case 'tool_call':
      return {
        sessionUpdate: 'tool_call',
        toolCallId: `legacy:${row.chunkId}`,
        title: text,
        status: 'completed',
      } as ChunkRow['update']

    case 'tool_result':
      return {
        sessionUpdate: 'tool_call_update',
        toolCallId: `legacy:${row.chunkId}`,
        status: 'completed',
      } as ChunkRow['update']

    case 'error':
    case 'stop':
    case 'text':
    default:
      return {
        sessionUpdate: 'agent_message_chunk',
        content: { type: 'text', text },
      } as ChunkRow['update']
  }
}

function attachSubscribe<T extends object>(collection: ObservableCollection<T>): void {
  Object.defineProperty(collection, 'subscribe', {
    configurable: true,
    enumerable: false,
    writable: false,
    value(callback: (rows: T[]) => void) {
      callback(collection.toArray)
      return collection.subscribeChanges(() => {
        callback(collection.toArray)
      })
    },
  })
}
