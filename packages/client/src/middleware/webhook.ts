import type {
  DurableSubscriberEventSelector,
  DurableSubscriberKeyStrategy,
  DurableSubscriberRetryPolicy,
  DurableSubscriberSecretRef,
  WebhookMiddleware,
} from '../types.js'

import { durableSubscriber } from './shared.js'

export interface WebhookOptions {
  readonly name?: string
  readonly target?: string
  readonly url?: string
  readonly events: readonly DurableSubscriberEventSelector[]
  readonly keyBy?: Exclude<DurableSubscriberKeyStrategy, 'session'>
  readonly headers?: Readonly<Record<string, DurableSubscriberSecretRef>>
  readonly retry?: DurableSubscriberRetryPolicy
}

/**
 * Builds declarative config for the Rust `WebhookSubscriber` active
 * durable-subscriber profile.
 *
 * Phase 3 gives this helper a real lowering target: the current host-side
 * Rust config requires a concrete delivery URL, so target-only routing is not
 * lowered yet.
 *
 * @example `const mw = webhook({ target: 'slack-approvals', url: 'https://hooks.slack.com/services/demo', events: ['permission_request'], keyBy: 'session_request' })`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export function webhook(options: WebhookOptions): WebhookMiddleware {
  if (!options.url) {
    throw new Error(
      'webhook middleware currently requires url for live lowering; target-only routing is pending host target config support',
    )
  }

  return durableSubscriber({
    kind: 'webhook',
    ...options,
  })
}
