import { createLiveQueryCollection } from '@tanstack/db'
import type { Collection } from '@tanstack/db'

import { promptRequestRowKey, type PromptRequestRow } from '../schema.js'

interface PromptRequestSourceOptions {
  promptRequests?: Collection<PromptRequestRow, string>
  promptTurns?: Collection<PromptRequestRow, string>
}

export function createQueuedTurnsCollection(
  opts: PromptRequestSourceOptions,
): Collection<PromptRequestRow, string> {
  const promptRequests = resolvePromptRequests(opts)
  return createLiveQueryCollection({
    query: (q: any) =>
      q
        .from({ t: promptRequests })
        .orderBy(({ t }: { t: PromptRequestRow }) => t.position ?? 0, 'asc')
        .fn.where(({ t }: { t: PromptRequestRow }) => t.state === 'queued'),
    getKey: (row: PromptRequestRow) => promptRequestRowKey(row),
  }) as unknown as Collection<PromptRequestRow, string>
}

function resolvePromptRequests(
  opts: PromptRequestSourceOptions,
): Collection<PromptRequestRow, string> {
  const promptRequests = opts.promptRequests ?? opts.promptTurns
  if (!promptRequests) {
    throw new Error('createQueuedTurnsCollection requires promptRequests')
  }
  return promptRequests
}
