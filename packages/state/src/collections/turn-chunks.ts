import { createLiveQueryCollection } from '@tanstack/db'
import type { Collection } from '@tanstack/db'

import type { ChunkRow } from '../schema.js'

export interface TurnChunksOptions {
  chunks: Collection<ChunkRow>
  promptTurnId: string
}

/**
 * Reactive view over all chunks emitted inside one prompt turn, ordered by
 * `seq` ascending so consumers can reconstruct the turn's output in order.
 */
export function createTurnChunksCollection(
  opts: TurnChunksOptions,
): Collection<ChunkRow> {
  const { promptTurnId } = opts
  return createLiveQueryCollection({
    query: (q: any) =>
      q
        .from({ c: opts.chunks })
        .orderBy(({ c }: { c: ChunkRow }) => c.seq, 'asc')
        .fn.where(({ c }: { c: ChunkRow }) => c.promptTurnId === promptTurnId),
    getKey: (row: ChunkRow) => row.chunkId,
  }) as unknown as Collection<ChunkRow>
}
