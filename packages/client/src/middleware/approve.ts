import type { ApproveMiddleware } from '../types.js'

import { cloneDefined } from './shared.js'

/**
 * Builds an approval middleware spec for prompt- or tool-scoped approval gates.
 *
 * This remains the declarative surface for the Rust `ApprovalGateSubscriber`
 * passive durable-subscriber profile.
 *
 * @example `const mw = approve({ scope: 'tool_calls', timeoutMs: 60_000 })`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export function approve(options: {
  readonly scope: 'tool_calls' | 'all'
  readonly timeoutMs?: number
}): ApproveMiddleware {
  return {
    kind: 'approve',
    ...cloneDefined(options),
  }
}
