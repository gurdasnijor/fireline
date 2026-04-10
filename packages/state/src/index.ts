/**
 * @fireline/state
 *
 * Fireline state schema and derived collection helpers.
 *
 * This package owns the canonical schema for what flows on the
 * Fireline durable stream. The Rust side validates its output
 * against the JSON Schema this package emits — see
 * `scripts/emit-schema.ts` and `dist/schema.json`.
 *
 * Exports:
 * - `firelineSchema` — the `createStateSchema` instance describing
 *   the wire format
 * - `createFirelineDB` — the factory function that wraps
 *   `createStreamDB` from `@durable-streams/state` with the Fireline
 *   schema
 * - Derived collection helpers — TanStack DB live queries that
 *   project the raw `messages` collection into useful entity views
 *   (prompt turns, chunks, sessions, etc.)
 */

export * from './schema.js'
// TODO: export * from './factory.js'
// TODO: export * from './collections/index.js'
