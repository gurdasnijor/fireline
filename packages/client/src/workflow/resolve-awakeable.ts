import { DurableStream, stream } from '@durable-streams/client'

import type { CompletionKey, WorkflowTraceContext } from './keys.js'
import { completionKeyStorageKey } from './keys.js'

type StreamEnvelope = {
  readonly type: string
  readonly key: string
  readonly headers?: Readonly<Record<string, unknown>>
  readonly value?: Record<string, unknown>
}

class ResolveGuard {
  private locked = false
  private readonly waiters: Array<() => void> = []

  async acquire(): Promise<void> {
    if (!this.locked) {
      this.locked = true
      return
    }

    await new Promise<void>((resolve) => {
      this.waiters.push(resolve)
    })
  }

  release(): void {
    const next = this.waiters.shift()
    if (next) {
      next()
      return
    }

    this.locked = false
  }

  isIdle(): boolean {
    return !this.locked && this.waiters.length === 0
  }
}

// Process-local coordination only; the durable stream remains the source of
// truth via `hasTerminalCompletion(...)`.
const resolveGuards = new Map<string, ResolveGuard>()

/**
 * Error returned when an awakeable completion already exists on the durable
 * stream for the same canonical completion key.
 *
 * @example `if (error instanceof AwakeableAlreadyResolvedError) { ... }`
 *
 * @remarks Anthropic primitive: Session.
 */
export class AwakeableAlreadyResolvedError extends Error {
  readonly key: CompletionKey

  constructor(key: CompletionKey) {
    super(`awakeable '${completionKeyStorageKey(key)}' is already resolved`)
    this.name = 'AwakeableAlreadyResolvedError'
    this.key = key
  }
}

/**
 * Options for appending a generic awakeable completion envelope.
 *
 * @example `await resolveAwakeable({ streamUrl, key, value: true })`
 *
 * @remarks Anthropic primitive: Session.
 */
export interface ResolveAwakeableOptions<T> {
  readonly streamUrl: string
  readonly key: CompletionKey
  readonly value: T
  readonly traceContext?: WorkflowTraceContext
  readonly headers?: Readonly<Record<string, string>>
}

/**
 * Options for appending a generic awakeable rejection envelope.
 *
 * @example `await rejectAwakeable({ streamUrl, key, error: { reason: 'denied' } })`
 *
 * @remarks Anthropic primitive: Session.
 */
export interface RejectAwakeableOptions<E> {
  readonly streamUrl: string
  readonly key: CompletionKey
  readonly error: E
  readonly traceContext?: WorkflowTraceContext
  readonly headers?: Readonly<Record<string, string>>
}

/**
 * Appends the canonical `awakeable_resolved` envelope for a completion key.
 *
 * The durable stream remains the source of truth for whether a key has already
 * been resolved. The helper performs a catch-up read first and rejects on a
 * duplicate completion instead of appending a second terminal envelope.
 *
 * @example `await resolveAwakeable({ streamUrl, key, value: { approved: true }, traceContext: { traceparent } })`
 *
 * @remarks Anthropic primitive: Session.
 */
export async function resolveAwakeable<T>(
  options: ResolveAwakeableOptions<T>,
): Promise<void> {
  await withResolveGuard(options.streamUrl, options.key, async () => {
    if (await hasTerminalCompletion(options.streamUrl, options.key, options.headers)) {
      throw new AwakeableAlreadyResolvedError(options.key)
    }

    const handle = new DurableStream({
      url: options.streamUrl,
      headers: options.headers,
      contentType: 'application/json',
    })

    await handle.append(JSON.stringify(awakeableResolutionEnvelope(options)), {
      contentType: 'application/json',
    })
  })
}

/**
 * Appends the canonical `awakeable_rejected` envelope for a completion key.
 *
 * The durable stream remains the source of truth for whether a key has already
 * been completed. Rejections and resolutions share the same first-wins
 * terminality.
 *
 * @example `await rejectAwakeable({ streamUrl, key, error: { reason: 'denied' } })`
 *
 * @remarks Anthropic primitive: Session.
 */
export async function rejectAwakeable<E>(
  options: RejectAwakeableOptions<E>,
): Promise<void> {
  await withResolveGuard(options.streamUrl, options.key, async () => {
    if (await hasTerminalCompletion(options.streamUrl, options.key, options.headers)) {
      throw new AwakeableAlreadyResolvedError(options.key)
    }

    const handle = new DurableStream({
      url: options.streamUrl,
      headers: options.headers,
      contentType: 'application/json',
    })

    await handle.append(JSON.stringify(awakeableRejectionEnvelope(options)), {
      contentType: 'application/json',
    })
  })
}

export function awakeableResolutionEnvelope<T>(
  options: ResolveAwakeableOptions<T>,
): StreamEnvelope {
  const baseValue: Record<string, unknown> = {
    kind: 'awakeable_resolved',
    sessionId: options.key.sessionId,
    value: options.value,
    resolvedAtMs: Date.now(),
  }

  if (options.key.kind === 'prompt') {
    baseValue.requestId = options.key.requestId
  } else if (options.key.kind === 'tool') {
    baseValue.toolCallId = options.key.toolCallId
  }

  const meta = traceContextMeta(options.traceContext)
  if (meta) {
    baseValue._meta = meta
  }

  return {
    type: 'awakeable',
    key: `${completionKeyStorageKey(options.key)}:resolved`,
    headers: { operation: 'insert' },
    value: baseValue,
  }
}

export function awakeableRejectionEnvelope<E>(
  options: RejectAwakeableOptions<E>,
): StreamEnvelope {
  const baseValue: Record<string, unknown> = {
    kind: 'awakeable_rejected',
    sessionId: options.key.sessionId,
    error: options.error,
    rejectedAtMs: Date.now(),
  }

  if (options.key.kind === 'prompt') {
    baseValue.requestId = options.key.requestId
  } else if (options.key.kind === 'tool') {
    baseValue.toolCallId = options.key.toolCallId
  }

  const meta = traceContextMeta(options.traceContext)
  if (meta) {
    baseValue._meta = meta
  }

  return {
    type: 'awakeable',
    key: `${completionKeyStorageKey(options.key)}:rejected`,
    headers: { operation: 'insert' },
    value: baseValue,
  }
}

async function hasTerminalCompletion(
  streamUrl: string,
  key: CompletionKey,
  headers?: Readonly<Record<string, string>>,
): Promise<boolean> {
  const response = await stream<StreamEnvelope>({
    url: streamUrl,
    headers,
    json: true,
    live: false,
    offset: '-1',
  })
  const rows = (await response.json()) as StreamEnvelope[]
  return rows.some((row) => matchesResolvedEnvelope(row, key) || matchesRejectedEnvelope(row, key))
}

function matchesResolvedEnvelope(
  row: StreamEnvelope,
  key: CompletionKey,
): boolean {
  return (
    row.type === 'awakeable' &&
    row.value?.kind === 'awakeable_resolved' &&
    row.key === `${completionKeyStorageKey(key)}:resolved`
  )
}

function matchesRejectedEnvelope(
  row: StreamEnvelope,
  key: CompletionKey,
): boolean {
  return (
    row.type === 'awakeable' &&
    row.value?.kind === 'awakeable_rejected' &&
    row.key === `${completionKeyStorageKey(key)}:rejected`
  )
}

function traceContextMeta(
  traceContext: WorkflowTraceContext | undefined,
): Record<string, string> | undefined {
  if (!traceContext) {
    return undefined
  }

  const meta = Object.fromEntries(
    Object.entries(traceContext).filter(([, value]) => value !== undefined && value !== ''),
  ) as Record<string, string>

  return Object.keys(meta).length > 0 ? meta : undefined
}

async function withResolveGuard<T>(
  streamUrl: string,
  key: CompletionKey,
  operation: () => Promise<T>,
): Promise<T> {
  const guardKey = `${streamUrl}::${completionKeyStorageKey(key)}`
  let guard = resolveGuards.get(guardKey)
  if (!guard) {
    guard = new ResolveGuard()
    resolveGuards.set(guardKey, guard)
  }

  await guard.acquire()
  try {
    return await operation()
  } finally {
    guard.release()
    if (guard.isIdle()) {
      resolveGuards.delete(guardKey)
    }
  }
}
