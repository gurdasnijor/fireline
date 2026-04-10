export { firelineState } from './schema.js'
export type { StateEvent } from '@durable-streams/state'

export {
  createFirelineDB,
  type FirelineDB,
  type FirelineDBConfig,
  type FirelineCollections,
} from './collection.js'

export type {
  ConnectionRow,
  PromptTurnRow,
  PendingRequestRow,
  PermissionRow,
  TerminalRow,
  RuntimeInstanceRow,
  SessionRow,
  ChildSessionEdgeRow,
  ChunkRow,
  ConnectionStatus,
} from './schema.js'

export {
  createQueuedTurnsCollection,
  createActiveTurnsCollection,
  createPendingPermissionsCollection,
  createSessionTurnsCollection,
  createConnectionTurnsCollection,
  createTurnChunksCollection,
  createSessionPermissionsCollection,
  type SessionTurnsOptions,
  type ConnectionTurnsOptions,
  type TurnChunksOptions,
  type SessionPermissionsOptions,
} from './collections/index.js'
