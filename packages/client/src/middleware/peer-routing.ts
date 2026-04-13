import type { PeerRoutingMiddleware } from '../types.js'

import { durableSubscriber } from './shared.js'

export interface PeerRoutingOptions {
  readonly name?: string
}

/**
 * Builds declarative config for the Rust `PeerRoutingSubscriber` active
 * durable-subscriber profile.
 *
 * @example `const mw = peerRouting()`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export function peerRouting(
  options: PeerRoutingOptions = {},
): PeerRoutingMiddleware {
  return durableSubscriber({
    kind: 'peerRouting',
    ...options,
  })
}
