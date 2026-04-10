import { createStateSchema } from '@durable-streams/state'
import { z } from 'zod'

export const connectionSchema = z.object({
  logicalConnectionId: z.string(),
  state: z.enum(['created', 'attached', 'broken', 'closed']),
  latestSessionId: z.string().optional(),
  lastError: z.string().optional(),
  queuePaused: z.boolean().optional(),
  createdAt: z.number(),
  updatedAt: z.number(),
})

export const promptTurnSchema = z.object({
  promptTurnId: z.string(),
  logicalConnectionId: z.string(),
  sessionId: z.string(),
  requestId: z.string(),
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
  requestId: z.string(),
  logicalConnectionId: z.string(),
  sessionId: z.string().optional(),
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
  requestId: z.string(),
  jsonrpcId: z.union([z.string(), z.number()]),
  logicalConnectionId: z.string(),
  sessionId: z.string(),
  promptTurnId: z.string(),
  title: z.string().optional(),
  toolCallId: z.string().optional(),
  options: z.array(permissionOptionSchema).optional(),
  state: z.enum(['pending', 'resolved', 'orphaned']),
  outcome: z.string().optional(),
  createdAt: z.number(),
  resolvedAt: z.number().optional(),
})

export const terminalSchema = z.object({
  terminalId: z.string(),
  logicalConnectionId: z.string(),
  sessionId: z.string(),
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

export const chunkSchema = z.object({
  chunkId: z.string(),
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

  chunks: {
    schema: chunkSchema,
    type: 'chunk',
    primaryKey: 'chunkId',
  },
})
