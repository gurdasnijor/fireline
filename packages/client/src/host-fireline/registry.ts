/**
 * Fireline-specific SessionRegistry satisfier for the shared state stream,
 * per `docs/proposals/client-primitives.md` and the managed-agent substrate
 * mapping in `docs/explorations/managed-agents-mapping.md`.
 */
import type { SessionRegistry } from '../orchestration/index.js'

export interface CreateStreamSessionRegistryOptions {
  readonly sharedStateUrl: string
}

export function createStreamSessionRegistry(
  opts: CreateStreamSessionRegistryOptions,
): SessionRegistry {
  void opts.sharedStateUrl

  return {
    async *listPending() {
      return
    },

    onPendingChange() {
      return () => {}
    },
  }
}
