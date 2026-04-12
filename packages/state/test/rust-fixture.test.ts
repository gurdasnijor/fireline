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
  connection: connectionSchema.strict(),
  pending_request: pendingRequestSchema.strict(),
  prompt_turn: promptTurnSchema.strict(),
  runtime_instance: runtimeInstanceSchema.strict(),
  session: sessionSchema.strict(),
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

      // ACP canonical-identifiers Phase 3 deletes the child_session_edge writer first.
      // Keep this fixture parser tolerant until the Rust fixture is regenerated
      // without that transitional write-only entity.
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
