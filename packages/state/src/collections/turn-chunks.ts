import { createLiveQueryCollection } from '@tanstack/db'
import type { Collection } from '@tanstack/db'

import type { ChunkRow } from '../schema.js'

export interface TurnChunksOptions {
  chunks: Collection<ChunkRow>
  requestId: string | number | null
}

/**
 * Reactive view over all canonical chunks emitted for one ACP request.
 * The durable stream append order is the source of truth; createdAt is the
 * best available stable sort key inside the materialized row value.
 */
export function createTurnChunksCollection(
  opts: TurnChunksOptions,
): Collection<ChunkRow> {
  const { requestId } = opts
  return createLiveQueryCollection({
    query: (q: any) =>
      q
        .from({ c: opts.chunks })
        .orderBy(({ c }: { c: ChunkRow }) => c.createdAt, 'asc')
        .fn.where(({ c }: { c: ChunkRow }) => c.requestId === requestId),
    getKey: (row: ChunkRow) => row.chunkKey,
  }) as unknown as Collection<ChunkRow>
}
