import { createLiveQueryCollection } from '@tanstack/db'
import type { Collection } from '@tanstack/db'

import type { SessionId } from '../acp-types.js'
import { permissionRowKey, type PermissionRow } from '../schema.js'

export interface SessionPermissionsOptions {
  permissions: Collection<PermissionRow, string>
  sessionId: SessionId
}

/**
 * Reactive view over all permission prompts that belong to a single ACP
 * session. Includes every state (`pending`, `resolved`, `orphaned`) so
 * consumers can render history or narrow further with a downstream query.
 */
export function createSessionPermissionsCollection(
  opts: SessionPermissionsOptions,
): Collection<PermissionRow, string> {
  const { sessionId } = opts
  return createLiveQueryCollection({
    query: (q: any) =>
      q
        .from({ p: opts.permissions })
        .orderBy(({ p }: { p: PermissionRow }) => p.createdAt, 'asc')
        .fn.where(({ p }: { p: PermissionRow }) => p.sessionId === sessionId),
    getKey: (row: PermissionRow) => permissionRowKey(row),
  }) as unknown as Collection<PermissionRow, string>
}
