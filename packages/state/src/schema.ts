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
  .strict()

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
  .strict()

const chunkEventSchema = chunkSchema
  .extend({
    chunkKey: z.string(),
  })
  .strict()

const firelineCollectionDefinitions = {
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

export type PromptRequestRow = z.infer<typeof promptRequestSchema>
export type PermissionOptionRow = z.infer<typeof permissionOptionSchema>
export type PermissionRow = z.infer<typeof permissionSchema>
export type RuntimeInstanceRow = z.infer<typeof runtimeInstanceSchema>
export type SessionRow = z.infer<typeof sessionSchema>
export type ChunkRow = z.infer<typeof chunkSchema>
export type PromptRequestEventRow = z.infer<typeof promptRequestEventSchema>
export type PermissionEventRow = z.infer<typeof permissionEventSchema>
export type SessionEventRow = SessionRow
export type ChunkEventRow = z.infer<typeof chunkEventSchema>
export type ConnectionStatus = 'disconnected' | 'connecting' | 'connected' | 'error'
