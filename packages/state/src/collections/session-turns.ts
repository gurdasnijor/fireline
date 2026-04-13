import { createLiveQueryCollection } from '@tanstack/db'
import type { Collection } from '@tanstack/db'

import type { SessionId } from '../acp-types.js'
import { promptRequestRowKey, type PromptRequestRow } from '../schema.js'

export interface SessionPromptRequestsOptions {
  promptRequests: Collection<PromptRequestRow, string>
  sessionId: SessionId
}

/**
 * Reactive view over all prompt requests that belong to a single ACP session,
 * ordered by `startedAt` ascending.
 */
export function createSessionPromptRequestsCollection(
  opts: SessionPromptRequestsOptions,
): Collection<PromptRequestRow, string> {
  const { sessionId, promptRequests } = opts
  return createLiveQueryCollection({
    query: (q: any) =>
      q
        .from({ t: promptRequests })
        .orderBy(({ t }: { t: PromptRequestRow }) => t.startedAt, 'asc')
        .fn.where(({ t }: { t: PromptRequestRow }) => t.sessionId === sessionId),
    getKey: (row: PromptRequestRow) => promptRequestRowKey(row),
  }) as unknown as Collection<PromptRequestRow, string>
}
