import { createLiveQueryCollection } from '@tanstack/db'
import type { Collection } from '@tanstack/db'

import type { RequestId, SessionId } from '../acp-types.js'
import { chunkRowKey, type ChunkRow } from '../schema.js'

export interface RequestChunksOptions {
  chunks: Collection<ChunkRow, string>
  sessionId?: SessionId
  requestId: RequestId
}

/**
 * Reactive view over all canonical chunks emitted for one ACP request.
 * The durable stream append order is the source of truth; createdAt is the
 * best available stable sort key inside the materialized row value.
 */
export function createRequestChunksCollection(
  opts: RequestChunksOptions,
): Collection<ChunkRow, string> {
  const { chunks, sessionId, requestId } = opts
  return createLiveQueryCollection({
    query: (q: any) =>
      q
        .from({ c: chunks })
        .orderBy(({ c }: { c: ChunkRow }) => c.createdAt, 'asc')
        .fn.where(
          ({ c }: { c: ChunkRow }) =>
            c.requestId === requestId &&
            (sessionId === undefined || c.sessionId === sessionId),
        ),
    getKey: (row: ChunkRow) => chunkRowKey(row),
  }) as unknown as Collection<ChunkRow, string>
}
