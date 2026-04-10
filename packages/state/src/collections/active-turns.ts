import { createLiveQueryCollection } from '@tanstack/db'
import type { Collection } from '@tanstack/db'

import type { PromptTurnRow } from '../schema.js'

export function createActiveTurnsCollection(
  opts: { promptTurns: Collection<PromptTurnRow> },
): Collection<PromptTurnRow> {
  return createLiveQueryCollection({
    query: (q: any) =>
      q
        .from({ t: opts.promptTurns })
        .fn.where(({ t }: { t: PromptTurnRow }) => t.state === 'active'),
    getKey: (row: PromptTurnRow) => row.promptTurnId,
  }) as unknown as Collection<PromptTurnRow>
}
