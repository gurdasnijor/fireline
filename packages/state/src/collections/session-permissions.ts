import { createLiveQueryCollection } from '@tanstack/db'
import type { Collection } from '@tanstack/db'

import type { PermissionRow } from '../schema.js'

export interface SessionPermissionsOptions {
  permissions: Collection<PermissionRow>
  sessionId: string
}

/**
 * Reactive view over all permission prompts that belong to a single ACP
 * session. Includes every state (`pending`, `resolved`, `orphaned`) so
 * consumers can render history or narrow further with a downstream query.
 */
export function createSessionPermissionsCollection(
  opts: SessionPermissionsOptions,
): Collection<PermissionRow> {
  const { sessionId } = opts
  return createLiveQueryCollection({
    query: (q: any) =>
      q
        .from({ p: opts.permissions })
        .orderBy(({ p }: { p: PermissionRow }) => p.createdAt, 'asc')
        .fn.where(({ p }: { p: PermissionRow }) => p.sessionId === sessionId),
    getKey: (row: PermissionRow) => row.requestId,
  }) as unknown as Collection<PermissionRow>
}
