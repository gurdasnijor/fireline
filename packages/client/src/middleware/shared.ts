import type {
  AutoApproveMiddleware,
  DurableSubscriberEventSelector,
  DurableSubscriberMiddleware,
  DurableSubscriberRetryPolicy,
  DurableSubscriberSecretRef,
  TelegramMiddleware,
  WebhookMiddleware,
} from '../types.js'

export function cloneDefined<T extends object>(value: T): T {
  return Object.fromEntries(
    Object.entries(value).filter(([, entry]) => entry !== undefined),
  ) as T
}

export function cloneEventSelectors(
  selectors: readonly DurableSubscriberEventSelector[] | undefined,
): readonly DurableSubscriberEventSelector[] | undefined {
  return selectors?.map((selector) =>
    typeof selector === 'string' ? selector : { ...selector },
  )
}

export function cloneRetryPolicy(
  retry: DurableSubscriberRetryPolicy | undefined,
): DurableSubscriberRetryPolicy | undefined {
  return retry ? { ...retry } : undefined
}

export function cloneSecretHeaders(
  headers: Readonly<Record<string, DurableSubscriberSecretRef>> | undefined,
): Readonly<Record<string, DurableSubscriberSecretRef>> | undefined {
  return headers
    ? Object.fromEntries(
        Object.entries(headers).map(([name, ref]) => [name, { ...ref }]),
      )
    : undefined
}

export function cloneTokenRef(
  token: string | DurableSubscriberSecretRef | undefined,
): string | DurableSubscriberSecretRef | undefined {
  return typeof token === 'string' || token === undefined ? token : { ...token }
}

/**
 * Identity helper for declarative durable-subscriber middleware specs.
 *
 * This stays intentionally thin in Phase 7: it normalizes the TypeScript shape
 * without asking callers to provide synthetic completion keys or imperative
 * callbacks.
 */
export function durableSubscriber<T extends DurableSubscriberMiddleware>(profile: T): T {
  switch (profile.kind) {
    case 'webhook':
      return {
        ...cloneDefined({
          name: profile.name,
          target: profile.target,
          url: profile.url,
          keyBy: profile.keyBy,
          headers: cloneSecretHeaders(profile.headers),
          retry: cloneRetryPolicy(profile.retry),
        }),
        kind: 'webhook',
        events: cloneEventSelectors(profile.events) ?? [],
      } as T
    case 'telegram':
      return {
        ...cloneDefined({
          name: profile.name,
          target: profile.target,
          token: cloneTokenRef(profile.token),
          chatId: profile.chatId,
          keyBy: profile.keyBy,
          retry: cloneRetryPolicy(profile.retry),
        }),
        kind: 'telegram',
        ...(profile.events ? { events: cloneEventSelectors(profile.events) } : {}),
      } as T
    case 'autoApprove':
      return {
        ...cloneDefined({
          name: profile.name,
          retry: cloneRetryPolicy(profile.retry),
        }),
        kind: 'autoApprove',
        ...(profile.events ? { events: cloneEventSelectors(profile.events) } : {}),
      } as T
  }
}

export type { AutoApproveMiddleware, TelegramMiddleware, WebhookMiddleware }
