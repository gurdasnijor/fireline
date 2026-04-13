import { createStreamDB, type StreamDB, type StreamDBMethods } from '@durable-streams/state'
import { createCollection, localOnlyCollectionOptions, type Collection } from '@tanstack/db'

import { promptRequestCollectionKey, type SessionUpdate } from './acp-types.js'
import { firelineStreamState } from './schema.js'
import type {
  ChunkEventRow,
  ChunkRow,
  PermissionEventRow,
  PermissionRow,
  PromptRequestEventRow,
  PromptRequestRow,
  SessionEventRow,
  SessionRow,
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
  promptRequests: ObservableCollection<PromptRequestRow>
  permissions: ObservableCollection<PermissionRow>
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
    promptRequests: createObservableLocalCollection((row) => promptRequestRowKey(row)),
    permissions: createObservableLocalCollection((row) => permissionRowKey(row)),
    sessions: createObservableLocalCollection((row) => row.sessionId),
    chunks: createObservableLocalCollection((row) => chunkRowKey(row)),
  }

  const syncAll = () => {
    reconcileCollection(
      collections.promptRequests,
      (rawDb.collections.promptRequests.toArray as PromptRequestEventRow[]).map(
        clonePromptRequestRow,
      ),
    )
    reconcileCollection(
      collections.permissions,
      projectPermissions(rawDb.collections.permissions.toArray as PermissionEventRow[]),
    )
    reconcileCollection(
      collections.sessions,
      (rawDb.collections.sessions.toArray as SessionEventRow[]).map(cloneSessionRow),
    )
    reconcileCollection(
      collections.chunks,
      (rawDb.collections.chunks.toArray as ChunkEventRow[]).map(cloneChunkRow),
    )
  }

  const unsubscribers = [
    rawDb.collections.promptRequests.subscribeChanges(syncAll),
    rawDb.collections.permissions.subscribeChanges(syncAll),
    rawDb.collections.sessions.subscribeChanges(syncAll),
    rawDb.collections.chunks.subscribeChanges(syncAll),
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

function clonePromptRequestRow(row: PromptRequestEventRow): PromptRequestRow {
  return withCollectionKey(
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
    row.promptRequestKey,
  )
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

function cloneSessionRow(row: SessionEventRow): SessionRow {
  return {
    sessionId: row.sessionId,
    state: row.state,
    supportsLoadSession: row.supportsLoadSession,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
    lastSeenAt: row.lastSeenAt,
  }
}

function cloneChunkRow(row: ChunkEventRow): ChunkRow {
  return withCollectionKey(
    {
      sessionId: row.sessionId,
      requestId: row.requestId,
      toolCallId: row.toolCallId,
      update: row.update as SessionUpdate,
      createdAt: row.createdAt,
    },
    row.chunkKey,
  )
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
