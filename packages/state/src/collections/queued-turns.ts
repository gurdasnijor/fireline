import { createLiveQueryCollection } from '@tanstack/db'
import type { Collection } from '@tanstack/db'

import { promptRequestRowKey, type PromptRequestRow } from '../schema.js'

export function createQueuedTurnsCollection(
  opts: { promptRequests: Collection<PromptRequestRow, string> },
): Collection<PromptRequestRow, string> {
  const { promptRequests } = opts
  return createLiveQueryCollection({
    query: (q: any) =>
      q
        .from({ t: promptRequests })
        .orderBy(({ t }: { t: PromptRequestRow }) => t.position ?? 0, 'asc')
        .fn.where(({ t }: { t: PromptRequestRow }) => t.state === 'queued'),
    getKey: (row: PromptRequestRow) => promptRequestRowKey(row),
  }) as unknown as Collection<PromptRequestRow, string>
}
