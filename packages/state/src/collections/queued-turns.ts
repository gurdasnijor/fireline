import { createLiveQueryCollection } from '@tanstack/db'
import type { Collection } from '@tanstack/db'

import type { PromptTurnRow } from '../schema.js'

export function createQueuedTurnsCollection(
  opts: { promptTurns: Collection<PromptTurnRow> },
): Collection<PromptTurnRow> {
  return createLiveQueryCollection({
    query: (q: any) =>
      q
        .from({ t: opts.promptTurns })
        .orderBy(({ t }: { t: PromptTurnRow }) => t.position ?? 0, 'asc')
        .fn.where(({ t }: { t: PromptTurnRow }) => t.state === 'queued'),
    getKey: (row: PromptTurnRow) => row.promptTurnId,
  }) as unknown as Collection<PromptTurnRow>
}
