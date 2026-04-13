import { createLiveQueryCollection } from '@tanstack/db'
import type { Collection } from '@tanstack/db'

import type { SessionId } from '../acp-types.js'
import { promptRequestRowKey, type PromptRequestRow } from '../schema.js'

interface PromptRequestSourceOptions {
  promptRequests?: Collection<PromptRequestRow, string>
  promptTurns?: Collection<PromptRequestRow, string>
}

export interface SessionPromptRequestsOptions extends PromptRequestSourceOptions {
  sessionId: SessionId
}

export type SessionTurnsOptions = SessionPromptRequestsOptions

/**
 * Reactive view over all prompt requests that belong to a single ACP session,
 * ordered by `startedAt` ascending.
 */
export function createSessionPromptRequestsCollection(
  opts: SessionPromptRequestsOptions,
): Collection<PromptRequestRow, string> {
  const { sessionId } = opts
  const promptRequests = resolvePromptRequests(opts)
  return createLiveQueryCollection({
    query: (q: any) =>
      q
        .from({ t: promptRequests })
        .orderBy(({ t }: { t: PromptRequestRow }) => t.startedAt, 'asc')
        .fn.where(({ t }: { t: PromptRequestRow }) => t.sessionId === sessionId),
    getKey: (row: PromptRequestRow) => promptRequestRowKey(row),
  }) as unknown as Collection<PromptRequestRow, string>
}

export const createSessionTurnsCollection = createSessionPromptRequestsCollection

function resolvePromptRequests(
  opts: PromptRequestSourceOptions,
): Collection<PromptRequestRow, string> {
  const promptRequests = opts.promptRequests ?? opts.promptTurns
  if (!promptRequests) {
    throw new Error('createSessionPromptRequestsCollection requires promptRequests')
  }
  return promptRequests
}
