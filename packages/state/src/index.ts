export { firelineState } from './schema.js'
export type { StateEvent } from '@durable-streams/state'
export type {
  SessionId,
  RequestId,
  ToolCallId,
  SessionUpdate,
  StopReason,
  PromptRequestRef,
  ToolInvocationRef,
} from './acp-types.js'
export { requestIdCollectionKey, promptRequestCollectionKey } from './acp-types.js'

export {
  createFirelineDB,
  type FirelineDB,
  type FirelineDBConfig,
  type FirelineCollections,
} from './collection.js'

export type {
  ConnectionRow,
  PromptRequestRow,
  PromptTurnRow,
  PermissionRow,
  TerminalRow,
  RuntimeInstanceRow,
  SessionRow,
  ChunkRow,
  ConnectionStatus,
} from './schema.js'

export {
  createQueuedTurnsCollection,
  createActiveTurnsCollection,
  createPendingPermissionsCollection,
  createSessionPromptRequestsCollection,
  createSessionTurnsCollection,
  createRequestChunksCollection,
  createTurnChunksCollection,
  createSessionPermissionsCollection,
  type SessionPromptRequestsOptions,
  type SessionTurnsOptions,
  type RequestChunksOptions,
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
