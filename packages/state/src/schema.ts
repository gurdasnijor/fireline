/**
 * Fireline wire format schema.
 *
 * Defines the shape of records that flow on the Fireline durable
 * stream. Each record is the SDK's `TraceEvent` (a JSON-RPC message
 * captured by the conductor's tracer) wrapped with two extension
 * fields: `runtimeId` and `observedAtMs`.
 *
 * The Rust side (`fireline_conductor::trace::DurableStreamTracer`)
 * produces records matching this schema. A conformance test in the
 * conductor crate validates this against the exported JSON Schema
 * artifact (`dist/schema.json`).
 *
 * The agent-facing entity views (prompt turns, chunks, sessions,
 * etc.) are NOT defined here. They live in `src/collections/*` as
 * derived TanStack DB queries that group/filter/transform raw
 * messages.
 */

// TODO: implement firelineSchema
//
// Target shape:
//
// ```ts
// import { createStateSchema } from '@durable-streams/state'
// import { z } from 'zod'
//
// const traceRecordSchema = z.object({
//   event: z.unknown(),  // SDK's TraceEvent (Request | Response | Notification)
//   runtimeId: z.string(),
//   observedAtMs: z.number(),
// })
//
// export const firelineSchema = createStateSchema({
//   messages: {
//     schema: traceRecordSchema,
//     type: 'trace_record',
//     primaryKey: 'id',
//   },
// })
// ```

export const firelineSchemaPlaceholder = 'TODO: implement firelineSchema'
