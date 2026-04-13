import { createLiveQueryCollection } from '@tanstack/db'
import type { Collection } from '@tanstack/db'

import { permissionRowKey, type PermissionRow } from '../schema.js'

export function createPendingPermissionsCollection(
  opts: { permissions: Collection<PermissionRow, string> },
): Collection<PermissionRow, string> {
  return createLiveQueryCollection({
    query: (q: any) =>
      q
        .from({ p: opts.permissions })
        .fn.where(({ p }: { p: PermissionRow }) => p.state === 'pending'),
    getKey: (row: PermissionRow) => permissionRowKey(row),
  }) as unknown as Collection<PermissionRow, string>
}
