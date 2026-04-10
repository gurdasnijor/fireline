/**
 * Fireline state schema.
 *
 * Defines the normalized entity collections that `@fireline/state`
 * materializes locally from the Fireline durable stream. The Rust
 * producer side emits STATE-PROTOCOL events (insert / update / delete)
 * into the stream; `@durable-streams/state` syncs those events into
 * the collections below.
 *
 * Each entry under `createStateSchema({...})` declares:
 * - `schema` — Zod definition for a row in that collection
 * - `type` — the STATE-PROTOCOL entity type tag stream events carry
 * - `primaryKey` — the row field used as the collection's key
 *
 * Row types are derived from the zod schemas via `z.infer` and
 * re-exported for use in collection typings and consumer code — the
 * zod schema is the single source of truth for both runtime
 * validation and compile-time types.
 *
 * Scope: intentionally narrowed to `chunks` for the first pass.
 * Additional entity collections (connections, prompt turns, pending
 * requests, permissions, terminals, runtime instances) will be
 * layered in as the producer side learns to emit each entity type.
 */

import { createStateSchema } from '@durable-streams/state'
import { z } from 'zod'

const chunkSchema = z.object({
  chunkId: z.string(),
  promptTurnId: z.string(),
  logicalConnectionId: z.string(),
  type: z.enum(['text', 'tool_call', 'thinking', 'tool_result', 'error', 'stop']),
  content: z.string(),
  seq: z.number(),
  createdAt: z.number(),
})

export type ChunkRow = z.infer<typeof chunkSchema>

export const firelineState = createStateSchema({
  chunks: {
    schema: chunkSchema,
    type: 'chunk',
    primaryKey: 'chunkId',
  },
})
