/**
 * @fireline/state
 *
 * Fireline state schema and stream-db factory.
 *
 * This package owns the canonical schema for what flows on the
 * Fireline durable stream. The Rust producer side emits
 * STATE-PROTOCOL events matching the zod schemas defined here;
 * consumers call `createFirelineDB()` to get a locally-materialized,
 * typed view backed by `@durable-streams/state` + `@tanstack/db`.
 */

// Schema + row types (single source of truth via z.infer)
export { firelineState, type ChunkRow } from './schema.js'
export type { StateEvent } from '@durable-streams/state'

// Stream-db factory
export {
  createFirelineDB,
  type FirelineDB,
  type FirelineDBConfig,
  type FirelineCollections,
} from './collection.js'
