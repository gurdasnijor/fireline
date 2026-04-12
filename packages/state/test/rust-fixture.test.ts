import { readFileSync } from 'node:fs'

import { describe, expect, it } from 'vitest'
import { z } from 'zod'

import {
  chunkSchema,
  connectionSchema,
  pendingRequestSchema,
  promptTurnSchema,
  runtimeInstanceSchema,
  sessionSchema,
} from '../src/schema.js'

const requestIdSchema = z.union([z.null(), z.string(), z.number().int()])

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

const strictValueSchemas = {
  chunk: chunkSchema.strict(),
  chunk_v2: z
    .object({
      sessionId: z.string(),
      requestId: requestIdSchema,
      toolCallId: z.string().optional(),
      type: z.enum(['text', 'tool_call', 'thinking', 'tool_result', 'error', 'stop']),
      content: z.string(),
      createdAt: z.number(),
    })
    .strict(),
  connection: connectionSchema.strict(),
  pending_request: pendingRequestSchema.strict(),
  prompt_request: z
    .object({
      sessionId: z.string(),
      requestId: requestIdSchema,
      text: z.string().optional(),
      state: z.enum(['active', 'completed', 'broken']),
      stopReason: z.string().optional(),
      startedAt: z.number(),
      completedAt: z.number().optional(),
    })
    .strict(),
  prompt_turn: promptTurnSchema.strict(),
  runtime_instance: runtimeInstanceSchema.strict(),
  session: sessionSchema.strict(),
  session_v2: z
    .object({
      sessionId: z.string(),
      runtimeKey: z.string(),
      runtimeId: z.string(),
      nodeId: z.string(),
      state: z.enum(['active', 'broken', 'closed']),
      supportsLoadSession: z.boolean(),
      createdAt: z.number(),
      updatedAt: z.number(),
      lastSeenAt: z.number(),
    })
    .strict(),
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

      // ACP canonical-identifiers Phase 3 deletes the child_session_edge writer
      // before the fixture is regenerated.
      if (parsed.type === 'child_session_edge') {
        continue
      }

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
