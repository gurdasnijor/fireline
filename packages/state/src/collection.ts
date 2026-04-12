import { createStreamDB, type StreamDB, type StreamDBMethods } from '@durable-streams/state'
import type { Collection } from '@tanstack/db'

import { firelineState } from './schema.js'
import type {
  ChildSessionEdgeRow,
  ChunkRow,
  ConnectionRow,
  PendingRequestRow,
  PermissionRow,
  PromptTurnRow,
  RuntimeInstanceRow,
  SessionRow,
  TerminalRow,
} from './schema.js'

export type ObservableCollection<T extends object> = Collection<T> & {
  subscribe(callback: (rows: T[]) => void): { unsubscribe(): void }
}

export interface FirelineCollections {
  connections: ObservableCollection<ConnectionRow>
  promptTurns: ObservableCollection<PromptTurnRow>
  pendingRequests: ObservableCollection<PendingRequestRow>
  permissions: ObservableCollection<PermissionRow>
  terminals: ObservableCollection<TerminalRow>
  runtimeInstances: ObservableCollection<RuntimeInstanceRow>
  sessions: ObservableCollection<SessionRow>
  childSessionEdges: ObservableCollection<ChildSessionEdgeRow>
  chunks: ObservableCollection<ChunkRow>
}

export type FirelineDB = {
  collections: FirelineCollections
} & StreamDBMethods

type RawFirelineDB = StreamDB<typeof firelineState>

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
    state: firelineState,
  })

  const db = rawDb as unknown as FirelineDB
  for (const collection of Object.values(db.collections)) {
    attachSubscribe(collection as ObservableCollection<object>)
  }
  return db
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
