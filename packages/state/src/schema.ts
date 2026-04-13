import { createStateSchema } from '@durable-streams/state'
import { z } from 'zod'
import {
  promptRequestCollectionKey,
  type RequestId,
  type SessionId,
  type SessionUpdate,
  type StopReason,
  type ToolCallId,
} from './acp-types.js'

const sessionIdSchema = z.custom<SessionId>(
  (value): value is SessionId => typeof value === 'string',
)

const requestIdSchema = z.custom<RequestId>(
  (value): value is RequestId =>
    value === null ||
    typeof value === 'string' ||
    (typeof value === 'number' && Number.isInteger(value)),
)

const toolCallIdSchema = z.custom<ToolCallId>(
  (value): value is ToolCallId => typeof value === 'string',
)

const sessionUpdateSchema = z.custom<SessionUpdate>(
  (value): value is SessionUpdate =>
    typeof value === 'object' &&
    value !== null &&
    'sessionUpdate' in value &&
    typeof (value as { sessionUpdate?: unknown }).sessionUpdate === 'string',
)

const stopReasonSchema = z.enum([
  'end_turn',
  'max_tokens',
  'max_turn_requests',
  'refusal',
  'cancelled',
]) as z.ZodType<StopReason>

const promptRequestStateSchema = z.enum([
  'queued',
  'active',
  'completed',
  'cancel_requested',
  'cancelled',
  'broken',
  'timed_out',
])

const permissionStateSchema = z.enum(['pending', 'resolved', 'orphaned'])

export const connectionSchema = z.object({
  logicalConnectionId: z.string(),
  state: z.enum(['created', 'attached', 'broken', 'closed']),
  latestSessionId: sessionIdSchema.optional(),
  lastError: z.string().optional(),
  queuePaused: z.boolean().optional(),
  createdAt: z.number(),
  updatedAt: z.number(),
})

export const promptRequestSchema = z.object({
  sessionId: sessionIdSchema,
  requestId: requestIdSchema,
  text: z.string().optional(),
  state: promptRequestStateSchema,
  position: z.number().optional(),
  stopReason: stopReasonSchema.optional(),
  startedAt: z.number(),
  completedAt: z.number().optional(),
})

export const permissionOptionSchema = z.object({
  optionId: z.string(),
  name: z.string(),
  kind: z.string(),
})

export const permissionSchema = z.object({
  sessionId: sessionIdSchema,
  requestId: requestIdSchema,
  title: z.string().optional(),
  toolCallId: toolCallIdSchema.optional(),
  options: z.array(permissionOptionSchema).optional(),
  state: permissionStateSchema,
  outcome: z.string().optional(),
  createdAt: z.number(),
  resolvedAt: z.number().optional(),
})

export const terminalSchema = z.object({
  terminalId: z.string(),
  logicalConnectionId: z.string(),
  sessionId: sessionIdSchema,
  promptTurnId: z.string().optional(),
  state: z.enum(['open', 'exited', 'released', 'broken']),
  command: z.string().optional(),
  exitCode: z.number().optional(),
  signal: z.string().optional(),
  createdAt: z.number(),
  updatedAt: z.number(),
})

export const runtimeInstanceSchema = z.object({
  instanceId: z.string(),
  runtimeName: z.string(),
  status: z.enum(['running', 'paused', 'stopped']),
  createdAt: z.number(),
  updatedAt: z.number(),
})

export const sessionSchema = z.object({
  sessionId: sessionIdSchema,
  state: z.enum(['active', 'broken', 'closed']),
  supportsLoadSession: z.boolean(),
  createdAt: z.number(),
  updatedAt: z.number(),
  lastSeenAt: z.number(),
})

export const chunkSchema = z.object({
  sessionId: sessionIdSchema,
  requestId: requestIdSchema,
  toolCallId: toolCallIdSchema.optional(),
  update: sessionUpdateSchema,
  createdAt: z.number(),
})

const promptRequestEventSchema = promptRequestSchema
  .extend({
    promptRequestKey: z.string(),
  })
  .passthrough()

const permissionEventSchema = z
  .object({
    permissionEventKey: z.string(),
    kind: z.enum(['permission_request', 'approval_resolved']),
    sessionId: sessionIdSchema,
    requestId: requestIdSchema.optional(),
    toolCallId: toolCallIdSchema.optional(),
    allow: z.boolean().optional(),
    resolvedBy: z.string().optional(),
    reason: z.string().optional(),
    createdAtMs: z.number(),
  })
  .passthrough()

const chunkEventSchema = chunkSchema
  .extend({
    chunkKey: z.string(),
  })
  .passthrough()

const legacyPromptTurnSchema = z
  .object({
    promptTurnId: z.string(),
    logicalConnectionId: z.string(),
    sessionId: sessionIdSchema,
    requestId: requestIdSchema,
    traceId: z.string().optional(),
    parentPromptTurnId: z.string().optional(),
    text: z.string().optional(),
    state: promptRequestStateSchema,
    position: z.number().optional(),
    stopReason: stopReasonSchema.optional(),
    startedAt: z.number(),
    completedAt: z.number().optional(),
  })
  .passthrough()

const legacySessionSchema = z
  .object({
    sessionId: sessionIdSchema,
    runtimeKey: z.string(),
    runtimeId: z.string(),
    nodeId: z.string(),
    logicalConnectionId: z.string(),
    state: z.enum(['active', 'broken', 'closed']),
    supportsLoadSession: z.boolean(),
    traceId: z.string().optional(),
    parentPromptTurnId: z.string().optional(),
    createdAt: z.number(),
    updatedAt: z.number(),
    lastSeenAt: z.number(),
  })
  .passthrough()

const legacyChunkSchema = z
  .object({
    chunkId: z.string(),
    sessionId: sessionIdSchema,
    promptTurnId: z.string(),
    logicalConnectionId: z.string(),
    type: z.enum(['text', 'tool_call', 'thinking', 'tool_result', 'error', 'stop']),
    content: z.string(),
    seq: z.number(),
    createdAt: z.number(),
  })
  .passthrough()

const firelineCollectionDefinitions = {
  connections: {
    schema: connectionSchema,
    type: 'connection',
    primaryKey: 'logicalConnectionId',
  },

  promptRequests: {
    schema: promptRequestEventSchema,
    type: 'prompt_request',
    primaryKey: 'promptRequestKey',
  },

  permissions: {
    schema: permissionEventSchema,
    type: 'permission',
    primaryKey: 'permissionEventKey',
  },

  terminals: {
    schema: terminalSchema,
    type: 'terminal',
    primaryKey: 'terminalId',
  },

  sessions: {
    schema: sessionSchema,
    type: 'session_v2',
    primaryKey: 'sessionId',
  },

  chunks: {
    schema: chunkEventSchema,
    type: 'chunk_v2',
    primaryKey: 'chunkKey',
  },
} as const

export const firelineState = createStateSchema(firelineCollectionDefinitions)

export const firelineStreamState = createStateSchema({
  ...firelineCollectionDefinitions,

  runtimeInstances: {
    schema: runtimeInstanceSchema,
    type: 'runtime_instance',
    primaryKey: 'instanceId',
  },

  legacyPromptTurns: {
    schema: legacyPromptTurnSchema,
    type: 'prompt_turn',
    primaryKey: 'promptTurnId',
  },

  legacySessions: {
    schema: legacySessionSchema,
    type: 'session',
    primaryKey: 'sessionId',
  },

  legacyChunks: {
    schema: legacyChunkSchema,
    type: 'chunk',
    primaryKey: 'chunkId',
  },
})

const COLLECTION_KEY = Symbol.for('fireline.state.collection_key')

type WithCollectionKey = {
  [COLLECTION_KEY]?: string
}

export function withCollectionKey<T extends object>(row: T, key: string): T {
  Object.defineProperty(row, COLLECTION_KEY, {
    configurable: true,
    enumerable: false,
    writable: false,
    value: key,
  })
  return row
}

export function collectionKeyOf(row: object): string | undefined {
  return (row as WithCollectionKey)[COLLECTION_KEY]
}

export function promptRequestRowKey(
  row: Pick<PromptRequestRow, 'sessionId' | 'requestId'>,
): string {
  return collectionKeyOf(row as object) ?? promptRequestCollectionKey(row.sessionId, row.requestId)
}

export function permissionRowKey(
  row: Pick<PermissionRow, 'sessionId' | 'requestId'>,
): string {
  return collectionKeyOf(row as object) ?? promptRequestCollectionKey(row.sessionId, row.requestId)
}

export function chunkRowKey(row: ChunkRow): string {
  return (
    collectionKeyOf(row) ??
    `${promptRequestCollectionKey(row.sessionId, row.requestId)}:${row.toolCallId ?? 'no_tool_call'}:${row.createdAt}`
  )
}

export type ConnectionRow = z.infer<typeof connectionSchema>
export type PromptRequestRow = z.infer<typeof promptRequestSchema>
export type PromptTurnRow = PromptRequestRow
export type PermissionOptionRow = z.infer<typeof permissionOptionSchema>
export type PermissionRow = z.infer<typeof permissionSchema>
export type TerminalRow = z.infer<typeof terminalSchema>
export type RuntimeInstanceRow = z.infer<typeof runtimeInstanceSchema>
export type SessionRow = z.infer<typeof sessionSchema>
export type ChunkRow = z.infer<typeof chunkSchema>
export type PromptRequestEventRow = z.infer<typeof promptRequestEventSchema>
export type PermissionEventRow = z.infer<typeof permissionEventSchema>
export type SessionEventRow = SessionRow
export type ChunkEventRow = z.infer<typeof chunkEventSchema>
export type LegacyPromptTurnEventRow = z.infer<typeof legacyPromptTurnSchema>
export type LegacySessionEventRow = z.infer<typeof legacySessionSchema>
export type LegacyChunkEventRow = z.infer<typeof legacyChunkSchema>
export type ConnectionStatus = 'disconnected' | 'connecting' | 'connected' | 'error'
