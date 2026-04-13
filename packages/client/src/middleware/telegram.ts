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
  readonly events?: readonly DurableSubscriberEventSelector[]
  readonly keyBy?: Exclude<DurableSubscriberKeyStrategy, 'session'>
  readonly retry?: DurableSubscriberRetryPolicy
}

/**
 * Builds declarative config for a Telegram-flavored active durable-subscriber
 * profile.
 *
 * This remains TS-only placeholder lowering until `mono-axr.11` lands the
 * corresponding Rust `TelegramSubscriber` profile.
 *
 * @example `const mw = telegram({ token: { ref: 'secret:telegram-bot' }, events: ['permission_request'] })`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export function telegram(options: TelegramOptions): TelegramMiddleware {
  if (!options.target && !options.token) {
    throw new Error('telegram middleware requires either target or token')
  }

  return durableSubscriber({
    kind: 'telegram',
    ...options,
    events: options.events ?? ['permission_request'],
    keyBy: options.keyBy ?? 'session_request',
  })
}
