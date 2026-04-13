import { readFileSync } from 'node:fs'

import { describe, expect, it } from 'vitest'
import { z } from 'zod'

import {
  chunkSchema,
  connectionSchema,
  runtimeInstanceSchema,
  sessionSchema,
} from '../src/schema.js'

const requestIdSchema = z.union([z.null(), z.string(), z.number().int()])
const stopReasonSchema = z.enum([
  'end_turn',
  'max_tokens',
  'max_turn_requests',
  'refusal',
  'cancelled',
])

const stateHeadersSchema = z
  .object({
    operation: z.enum(['insert', 'update', 'delete']),
  })
  .strict()

const baseEnvelopeSchema = z
  .object({
    type: z.string(),
    key: z.string(),
    headers: stateHeadersSchema,
    value: z.unknown().optional(),
  })
  .strict()

const legacyChunkSchema = z
  .object({
    chunkId: z.string(),
    sessionId: z.string(),
    promptTurnId: z.string(),
    logicalConnectionId: z.string(),
    type: z.enum(['text', 'tool_call', 'thinking', 'tool_result', 'error', 'stop']),
    content: z.string(),
    seq: z.number(),
    createdAt: z.number(),
  })
  .passthrough()

const legacyPromptTurnSchema = z
  .object({
    promptTurnId: z.string(),
    logicalConnectionId: z.string(),
    sessionId: z.string(),
    requestId: requestIdSchema,
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
    stopReason: stopReasonSchema.optional(),
    startedAt: z.number(),
    completedAt: z.number().optional(),
  })
  .passthrough()

const legacySessionSchema = z
  .object({
    sessionId: z.string(),
    runtimeKey: z.string(),
    runtimeId: z.string(),
    nodeId: z.string(),
    logicalConnectionId: z.string(),
    state: z.enum(['active', 'broken', 'closed']),
    supportsLoadSession: z.boolean(),
    createdAt: z.number(),
    updatedAt: z.number(),
    lastSeenAt: z.number(),
  })
  .passthrough()

const promptRequestEventValueSchema = z
  .object({
    sessionId: z.string(),
    requestId: requestIdSchema,
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
    stopReason: stopReasonSchema.optional(),
    startedAt: z.number(),
    completedAt: z.number().optional(),
  })
  .strict()

const permissionEventValueSchema = z
  .object({
    kind: z.enum(['permission_request', 'approval_resolved']),
    sessionId: z.string(),
    requestId: requestIdSchema.optional(),
    toolCallId: z.string().optional(),
    allow: z.boolean().optional(),
    resolvedBy: z.string().optional(),
    reason: z.string().optional(),
    createdAtMs: z.number(),
  })
  .strict()

const sessionEventValueSchema = sessionSchema
  .extend({
    runtimeKey: z.string(),
    runtimeId: z.string(),
    nodeId: z.string(),
  })
  .strict()

const chunkEventValueSchema = chunkSchema
  .extend({
    chunkKey: z.string(),
  })
  .strict()

const strictValueSchemas = {
  chunk: legacyChunkSchema,
  chunk_v2: chunkEventValueSchema,
  connection: connectionSchema.strict(),
  permission: permissionEventValueSchema,
  prompt_request: promptRequestEventValueSchema,
  prompt_turn: legacyPromptTurnSchema,
  runtime_instance: runtimeInstanceSchema.strict(),
  session: legacySessionSchema,
  session_v2: sessionEventValueSchema,
} as const

describe('Rust producer fixture', () => {
  it('strictly matches the Fireline state schema', () => {
    const fixture = readFileSync(
      new URL('./fixtures/rust-state-producer.ndjson', import.meta.url),
      'utf8',
    )

    const lines = fixture
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean)

    expect(lines.length).toBeGreaterThan(0)

    for (const line of lines) {
      const parsed = baseEnvelopeSchema.parse(JSON.parse(line))
      expect(parsed.type).not.toBe('child_session_edge')

      const valueSchema = strictValueSchemas[parsed.type as keyof typeof strictValueSchemas]

      expect(valueSchema, `unexpected entity type in Rust fixture: ${parsed.type}`).toBeDefined()

      if (!valueSchema) {
        continue
      }

      if (parsed.headers.operation === 'delete') {
        expect(parsed.value === undefined || parsed.value === null).toBe(true)
      } else {
        valueSchema.parse(parsed.value)
      }
    }
  })
})
