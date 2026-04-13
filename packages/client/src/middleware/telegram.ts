import type {
  DurableSubscriberEventSelector,
  DurableSubscriberKeyStrategy,
  DurableSubscriberRetryPolicy,
  DurableSubscriberSecretRef,
  TelegramMiddleware,
} from '../types.js'

import { durableSubscriber } from './shared.js'

export interface TelegramOptions {
  readonly name?: string
  readonly target?: string
  readonly token?: string | DurableSubscriberSecretRef
  readonly chatId?: string
  readonly allowedUserIds?: readonly string[]
  readonly scope?: 'tool_calls'
  readonly apiBaseUrl?: string
  readonly approvalTimeoutMs?: number
  readonly pollIntervalMs?: number
  readonly pollTimeoutMs?: number
  readonly parseMode?: 'html' | 'markdown_v2'
  readonly events?: readonly DurableSubscriberEventSelector[]
  readonly keyBy?: Exclude<DurableSubscriberKeyStrategy, 'session'>
  readonly retry?: DurableSubscriberRetryPolicy
}

/**
 * Builds declarative config for the Rust `TelegramSubscriber` active
 * durable-subscriber profile.
 *
 * The live lowering targets the `TelegramSubscriberConfig` shape introduced by
 * `mono-axr.11`. The older placeholder fields (`events`, `keyBy`, `retry`) are
 * still accepted at the type surface for compatibility, but they do not drive
 * the control-plane payload.
 *
 * @example `const mw = telegram({ token: 'env:TELEGRAM_BOT_TOKEN', scope: 'tool_calls' })`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export function telegram(options: TelegramOptions): TelegramMiddleware {
  if (!options.token) {
    throw new Error(
      'telegram middleware requires token for live lowering; target-only routing is not supported by TelegramSubscriberConfig',
    )
  }

  return durableSubscriber({
    kind: 'telegram',
    ...options,
    scope: options.scope ?? 'tool_calls',
    parseMode: options.parseMode ?? 'html',
    pollIntervalMs: options.pollIntervalMs ?? 1_000,
    pollTimeoutMs: options.pollTimeoutMs ?? 30_000,
  })
}
