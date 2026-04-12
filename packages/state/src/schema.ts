import { createStateSchema } from '@durable-streams/state'
import { z } from 'zod'
import type { RequestId, SessionId, ToolCallId } from './acp-types.js'

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

export const connectionSchema = z.object({
  logicalConnectionId: z.string(),
  state: z.enum(['created', 'attached', 'broken', 'closed']),
  latestSessionId: sessionIdSchema.optional(),
  lastError: z.string().optional(),
  queuePaused: z.boolean().optional(),
  createdAt: z.number(),
  updatedAt: z.number(),
})

export const promptTurnSchema = z.object({
  promptTurnId: z.string(),
  logicalConnectionId: z.string(),
  sessionId: sessionIdSchema,
  requestId: requestIdSchema,
  traceId: z.string().optional(),
  parentPromptTurnId: z.string().optional(),
  text: z.string().optional(),
  state: z.enum([
    'queued',
    'active',
    'completed',
    'cancel_requested',
    'cancelled',
    'broken',
    'timed_out',
  ]),
  position: z.number().optional(),
  stopReason: z.string().optional(),
  startedAt: z.number(),
  completedAt: z.number().optional(),
})

export const pendingRequestSchema = z.object({
  requestId: requestIdSchema,
  logicalConnectionId: z.string(),
  sessionId: sessionIdSchema.optional(),
  promptTurnId: z.string().optional(),
  method: z.string(),
  direction: z.enum(['client_to_agent', 'agent_to_client']),
  state: z.enum(['pending', 'resolved', 'orphaned']),
  createdAt: z.number(),
  resolvedAt: z.number().optional(),
})

export const permissionOptionSchema = z.object({
  optionId: z.string(),
  name: z.string(),
  kind: z.string(),
})

export const permissionSchema = z.object({
  requestId: requestIdSchema,
  jsonrpcId: requestIdSchema,
  logicalConnectionId: z.string(),
  sessionId: sessionIdSchema,
  promptTurnId: z.string(),
  title: z.string().optional(),
  toolCallId: toolCallIdSchema.optional(),
  options: z.array(permissionOptionSchema).optional(),
  state: z.enum(['pending', 'resolved', 'orphaned']),
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

export const childSessionEdgeSchema = z.object({
  edgeId: z.string(),
  traceId: z.string().optional(),
  parentRuntimeId: z.string(),
  parentSessionId: sessionIdSchema,
  parentPromptTurnId: z.string(),
  childRuntimeId: z.string(),
  childSessionId: sessionIdSchema,
  createdAt: z.number(),
})

export const chunkSchema = z.object({
  chunkId: z.string(),
  sessionId: sessionIdSchema,
  promptTurnId: z.string(),
  logicalConnectionId: z.string(),
  type: z.enum(['text', 'tool_call', 'thinking', 'tool_result', 'error', 'stop']),
  content: z.string(),
  seq: z.number(),
  createdAt: z.number(),
})

export type ConnectionRow = z.infer<typeof connectionSchema>
export type PromptTurnRow = z.infer<typeof promptTurnSchema>
export type PendingRequestRow = z.infer<typeof pendingRequestSchema>
export type PermissionOptionRow = z.infer<typeof permissionOptionSchema>
export type PermissionRow = z.infer<typeof permissionSchema>
export type TerminalRow = z.infer<typeof terminalSchema>
export type RuntimeInstanceRow = z.infer<typeof runtimeInstanceSchema>
export type SessionRow = z.infer<typeof sessionSchema>
export type ChildSessionEdgeRow = z.infer<typeof childSessionEdgeSchema>
export type ChunkRow = z.infer<typeof chunkSchema>
export type ConnectionStatus = 'disconnected' | 'connecting' | 'connected' | 'error'

export const firelineState = createStateSchema({
  connections: {
    schema: connectionSchema,
    type: 'connection',
    primaryKey: 'logicalConnectionId',
  },

  promptTurns: {
    schema: promptTurnSchema,
    type: 'prompt_turn',
    primaryKey: 'promptTurnId',
  },

  pendingRequests: {
    schema: pendingRequestSchema,
    type: 'pending_request',
    primaryKey: 'requestId',
  },

  permissions: {
    schema: permissionSchema,
    type: 'permission',
    primaryKey: 'requestId',
  },

  terminals: {
    schema: terminalSchema,
    type: 'terminal',
    primaryKey: 'terminalId',
  },

  runtimeInstances: {
    schema: runtimeInstanceSchema,
    type: 'runtime_instance',
    primaryKey: 'instanceId',
  },

  sessions: {
    schema: sessionSchema,
    type: 'session',
    primaryKey: 'sessionId',
  },

  childSessionEdges: {
    schema: childSessionEdgeSchema,
    type: 'child_session_edge',
    primaryKey: 'edgeId',
  },

  chunks: {
    schema: chunkSchema,
    type: 'chunk',
    primaryKey: 'chunkId',
  },
})
