import { createLiveQueryCollection } from '@tanstack/db'
import type { Collection } from '@tanstack/db'

import type { PromptTurnRow } from '../schema.js'

export interface SessionTurnsOptions {
  promptTurns: Collection<PromptTurnRow>
  sessionId: string
}

/**
 * Reactive view over all prompt turns that belong to a single ACP session,
 * ordered by `startedAt` ascending.
 */
export function createSessionTurnsCollection(
  opts: SessionTurnsOptions,
): Collection<PromptTurnRow> {
  const { sessionId } = opts
  return createLiveQueryCollection({
    query: (q: any) =>
      q
        .from({ t: opts.promptTurns })
        .orderBy(({ t }: { t: PromptTurnRow }) => t.startedAt, 'asc')
        .fn.where(({ t }: { t: PromptTurnRow }) => t.sessionId === sessionId),
    getKey: (row: PromptTurnRow) => row.promptTurnId,
  }) as unknown as Collection<PromptTurnRow>
}
