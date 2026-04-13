import { DurableStream, stream } from '@durable-streams/client'

import type { SessionId } from '../acp-ids.js'
import type { CompletionKey } from './keys.js'
import { completionKeyStorageKey, sessionCompletionKey } from './keys.js'

type StreamEnvelope = {
  readonly type: string
  readonly key: string
  readonly headers?: Readonly<Record<string, unknown>>
  readonly value?: Record<string, unknown>
}

/**
 * Imperative handle for a passive durable-subscriber wait.
 *
 * @example `const approval = ctx.awakeable<boolean>(key); const allowed = await approval.promise`
 *
 * @remarks Anthropic primitive: Session.
 */
export interface Awakeable<T> {
  readonly key: CompletionKey
  readonly promise: Promise<T>
}

/**
 * Options for constructing a workflow context bound to a Fireline state
 * stream.
 *
 * @example `const ctx = workflowContext({ stateStreamUrl })`
 *
 * @remarks Anthropic primitive: Session.
 */
export interface WorkflowContextOptions {
  readonly stateStreamUrl: string
  readonly headers?: Readonly<Record<string, string>>
}

/**
 * Minimal TypeScript workflow context for durable awakeable waits.
 *
 * This mirrors the Rust `WorkflowContext`: it binds a state stream URL and
 * exposes `ctx.awakeable<T>(key)` as imperative sugar over the passive
 * durable-subscriber substrate.
 *
 * @example `const ctx = new WorkflowContext({ stateStreamUrl })`
 *
 * @remarks Anthropic primitive: Session.
 */
export class WorkflowContext {
  readonly stateStreamUrl: string
  readonly headers?: Readonly<Record<string, string>>

  constructor(options: WorkflowContextOptions) {
    this.stateStreamUrl = options.stateStreamUrl
    this.headers = options.headers
  }

  awakeable<T>(key: CompletionKey): Awakeable<T> {
    return {
      key,
      promise: waitForAwakeable<T>({
        stateStreamUrl: this.stateStreamUrl,
        headers: this.headers,
        key,
      }),
    }
  }

  sessionAwakeable<T>(sessionId: SessionId): Awakeable<T> {
    return this.awakeable<T>(sessionCompletionKey(sessionId))
  }
}

/**
 * Factory wrapper for the TypeScript workflow context.
 *
 * @example `const ctx = workflowContext({ stateStreamUrl })`
 *
 * @remarks Anthropic primitive: Session.
 */
export function workflowContext(options: WorkflowContextOptions): WorkflowContext {
  return new WorkflowContext(options)
}

async function waitForAwakeable<T>(options: {
  readonly stateStreamUrl: string
  readonly headers?: Readonly<Record<string, string>>
  readonly key: CompletionKey
}): Promise<T> {
  const handle = new DurableStream({
    url: options.stateStreamUrl,
    headers: options.headers,
    contentType: 'application/json',
  })

  await handle.append(JSON.stringify(awakeableWaitingEnvelope(options.key)), {
    contentType: 'application/json',
  })

  const response = await stream<StreamEnvelope>({
    url: options.stateStreamUrl,
    headers: options.headers,
    json: true,
    live: true,
    offset: '-1',
  })

  return await new Promise<T>((resolve, reject) => {
    let settled = false
    let stopAssigned = false
    let pendingStop = false
    let stop = () => {
      pendingStop = true
    }

    const finish = (callback: () => void) => {
      if (settled) {
        return
      }
      settled = true
      stop()
      callback()
    }

    stop = response.subscribeJson((batch) => {
      for (const row of batch.items) {
        if (matchesResolvedEnvelope(row, options.key)) {
          finish(() => {
            resolve(row.value?.value as T)
          })
          return
        }
        if (matchesRejectedEnvelope(row, options.key)) {
          finish(() => {
            reject(
              new Error(
                `awakeable '${completionKeyStorageKey(options.key)}' rejected: ${JSON.stringify(row.value?.error ?? null)}`,
              ),
            )
          })
          return
        }
      }

      if (batch.streamClosed && batch.upToDate) {
        finish(() => {
          reject(
            new Error(
              `awakeable completion missing for key ${completionKeyStorageKey(options.key)}`,
            ),
          )
        })
      }
    })
    stopAssigned = true
    if (pendingStop) {
      stop()
    }
  })
}

function awakeableWaitingEnvelope(key: CompletionKey): StreamEnvelope {
  const baseValue: Record<string, unknown> = {
    kind: 'awakeable_waiting',
    sessionId: key.sessionId,
    createdAtMs: Date.now(),
  }

  if (key.kind === 'prompt') {
    baseValue.requestId = key.requestId
  } else if (key.kind === 'tool') {
    baseValue.toolCallId = key.toolCallId
  }

  return {
    type: 'awakeable',
    key: `${completionKeyStorageKey(key)}:waiting`,
    headers: { operation: 'insert' },
    value: baseValue,
  }
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
