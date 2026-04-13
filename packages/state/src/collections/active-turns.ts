import { createLiveQueryCollection } from '@tanstack/db'
import type { Collection } from '@tanstack/db'

import { promptRequestRowKey, type PromptRequestRow } from '../schema.js'

export function createActiveTurnsCollection(
  opts: { promptRequests: Collection<PromptRequestRow, string> },
): Collection<PromptRequestRow, string> {
  const { promptRequests } = opts
  return createLiveQueryCollection({
    query: (q: any) =>
      q
        .from({ t: promptRequests })
        .fn.where(({ t }: { t: PromptRequestRow }) => t.state === 'active'),
    getKey: (row: PromptRequestRow) => promptRequestRowKey(row),
  }) as unknown as Collection<PromptRequestRow, string>
}
