import { createLiveQueryCollection } from '@tanstack/db'
import type { Collection } from '@tanstack/db'

import type { PromptTurnRow } from '../schema.js'

export interface ConnectionTurnsOptions {
  promptTurns: Collection<PromptTurnRow>
  logicalConnectionId: string
}

/**
 * Reactive view over all prompt turns that belong to a single logical
 * connection, ordered by `startedAt` ascending. Spans every session the
 * connection has produced.
 */
export function createConnectionTurnsCollection(
  opts: ConnectionTurnsOptions,
): Collection<PromptTurnRow> {
  const { logicalConnectionId } = opts
  return createLiveQueryCollection({
    query: (q: any) =>
      q
        .from({ t: opts.promptTurns })
        .orderBy(({ t }: { t: PromptTurnRow }) => t.startedAt, 'asc')
        .fn.where(
          ({ t }: { t: PromptTurnRow }) => t.logicalConnectionId === logicalConnectionId,
        ),
    getKey: (row: PromptTurnRow) => row.promptTurnId,
  }) as unknown as Collection<PromptTurnRow>
}
