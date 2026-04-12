export { firelineState } from './schema.js'
export type { StateEvent } from '@durable-streams/state'
export type {
  SessionId,
  RequestId,
  ToolCallId,
  SessionUpdate,
  PromptRequestRef,
  ToolInvocationRef,
} from './acp-types.js'
export { requestIdCollectionKey } from './acp-types.js'

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

export {
  extractChunkTextPreview,
  isToolCallSessionUpdate,
  sessionUpdateKind,
  sessionUpdateStatus,
  sessionUpdateTitle,
  sessionUpdateToolCallId,
} from './session-updates.js'
