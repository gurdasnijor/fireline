/**
 * Fireline stream-db factory.
 *
 * Wraps `createStreamDB` from `@durable-streams/state` with the
 * Fireline schema to produce a typed, locally-materialized view of
 * the Fireline durable stream. The resulting `FirelineDB` exposes
 * typed collections (currently just `chunks`; more to come) that are
 * automatically populated from STATE-PROTOCOL events on the stream.
 */

import { createStreamDB, type StreamDB, type StreamDBMethods } from '@durable-streams/state'
import type { Collection } from '@tanstack/db'
import { firelineState, type ChunkRow } from './schema.js'

// ============================================================================
// FirelineDB Types
// ============================================================================

/**
 * Collections map with correct row types.
 */
export interface FirelineCollections {
  chunks: Collection<ChunkRow>
}

/**
 * Type alias for a Fireline stream-db instance.
 *
 * Provides typed access to all collections plus stream-db methods:
 * - `db.preload()` — wait for initial sync
 * - `db.close()` — cleanup resources
 * - `db.utils.awaitTxId(txid)` — wait for a specific write to sync
 */
export type FirelineDB = {
  collections: FirelineCollections
} & StreamDBMethods

/**
 * Internal type for the raw stream-db instance.
 * @internal
 */
type RawFirelineDB = StreamDB<typeof firelineState>

// ============================================================================
// Configuration
// ============================================================================

/**
 * Configuration for creating a Fireline stream-db.
 */
export interface FirelineDBConfig {
  /**
   * URL of the Fireline state stream.
   *
   * For a Fireline runtime with the embedded durable-streams-server
   * mounted on the default port, this is of the form
   * `http://localhost:4437/v1/stream/{name}` where `{name}` is the
   * name of the stream the runtime writes trace records to.
   */
  stateStreamUrl: string
  /** Additional headers for stream requests */
  headers?: Record<string, string>
  /** AbortSignal to cancel the stream sync */
  signal?: AbortSignal
}

// ============================================================================
// FirelineDB Factory
// ============================================================================

/**
 * Create a stream-db instance backed by a Fireline state stream.
 *
 * This function is synchronous — it creates the stream handle and
 * collections but does not start the stream connection. Call
 * `db.preload()` to connect and wait for the initial sync to complete.
 *
 * @example
 * ```typescript
 * const db = createFirelineDB({
 *   stateStreamUrl: 'http://localhost:4437/v1/stream/fireline-main',
 * })
 *
 * await db.preload()
 *
 * for (const chunk of db.collections.chunks.values()) {
 *   console.log(chunk.chunkId, chunk.type, chunk.content)
 * }
 *
 * db.close()
 * ```
 */
export function createFirelineDB(config: FirelineDBConfig): FirelineDB {
  const { stateStreamUrl, headers, signal } = config

  const rawDb: RawFirelineDB = createStreamDB({
    streamOptions: {
      url: stateStreamUrl,
      headers,
      signal,
    },
    state: firelineState,
  })

  // Cast to our FirelineDB type which has correctly typed collections
  return rawDb as unknown as FirelineDB
}
