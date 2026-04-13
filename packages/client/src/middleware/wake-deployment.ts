import type { WakeDeploymentMiddleware } from '../types.js'

import { durableSubscriber } from './shared.js'

export interface WakeDeploymentOptions {
  readonly name?: string
}

/**
 * Builds declarative config for the Rust `AlwaysOnDeploymentSubscriber`
 * durable-subscriber profile.
 *
 * @example `const mw = wakeDeployment()`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export function wakeDeployment(
  options: WakeDeploymentOptions = {},
): WakeDeploymentMiddleware {
  return durableSubscriber({
    kind: 'wakeDeployment',
    ...options,
  })
}
