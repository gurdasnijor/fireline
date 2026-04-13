import type {
  AutoApproveMiddleware,
  DurableSubscriberEventSelector,
  DurableSubscriberRetryPolicy,
} from '../types.js'

import { durableSubscriber } from './shared.js'

export interface AutoApproveOptions {
  readonly name?: string
  readonly events?: readonly DurableSubscriberEventSelector[]
  readonly retry?: DurableSubscriberRetryPolicy
}

/**
 * Builds declarative config for the Rust `AutoApproveSubscriber` active
 * durable-subscriber profile.
 *
 * Phase 4 gives this helper a live lowering target via the `auto_approve`
 * host topology component. The richer event/retry shape stays on the TS side
 * until the host exposes matching config knobs.
 *
 * @example `const mw = autoApprove()`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export function autoApprove(
  options: AutoApproveOptions = {},
): AutoApproveMiddleware {
  return durableSubscriber({
    kind: 'autoApprove',
    ...options,
    events: options.events ?? ['permission_request'],
  })
}
