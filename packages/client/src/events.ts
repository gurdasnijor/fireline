import { DurableStream } from '@durable-streams/client'

/**
 * Appends an external `approval_resolved` permission event to a Fireline state stream.
 *
 * @example `await appendApprovalResolved({ streamUrl, sessionId, requestId, allow: true })`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export async function appendApprovalResolved(options: {
  readonly streamUrl: string
  readonly sessionId: string
  readonly requestId: string
  readonly allow: boolean
  readonly resolvedBy?: string
}): Promise<void> {
  const stream = new DurableStream({ url: options.streamUrl })
  const envelope = {
    type: 'permission',
    key: `${options.sessionId}:${options.requestId}:resolved`,
    headers: { operation: 'insert' },
    value: {
      kind: 'approval_resolved',
      sessionId: options.sessionId,
      requestId: options.requestId,
      allow: options.allow,
      resolvedBy: options.resolvedBy ?? '@fireline/client',
      createdAtMs: Date.now(),
    },
  }
  await stream.append(JSON.stringify(envelope), { contentType: 'application/json' })
}
