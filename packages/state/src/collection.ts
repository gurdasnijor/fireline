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

export interface FirelineCollections {
  connections: Collection<ConnectionRow>
  promptTurns: Collection<PromptTurnRow>
  pendingRequests: Collection<PendingRequestRow>
  permissions: Collection<PermissionRow>
  terminals: Collection<TerminalRow>
  runtimeInstances: Collection<RuntimeInstanceRow>
  sessions: Collection<SessionRow>
  childSessionEdges: Collection<ChildSessionEdgeRow>
  chunks: Collection<ChunkRow>
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

  return rawDb as unknown as FirelineDB
}
