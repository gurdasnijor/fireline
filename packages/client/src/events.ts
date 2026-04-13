import { DurableStream } from '@durable-streams/client'
import type { RequestId, SessionId, ToolCallId } from './acp-ids.js'

type ApprovalResolutionKey =
  | {
      readonly requestId: RequestId
      readonly toolCallId?: never
    }
  | {
      readonly requestId?: never
      readonly toolCallId: ToolCallId
    }

/**
 * Appends an external `approval_resolved` permission event to a Fireline state stream.
 *
 * @example `await appendApprovalResolved({ streamUrl, sessionId, requestId, allow: true })`
 * @example `await appendApprovalResolved({ streamUrl, sessionId, toolCallId, allow: true })`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export async function appendApprovalResolved(options: {
  readonly streamUrl: string
  readonly sessionId: SessionId
  readonly allow: boolean
  readonly resolvedBy?: string
} & ApprovalResolutionKey): Promise<void> {
  const stream = new DurableStream({ url: options.streamUrl })
  const keySuffix = 'requestId' in options ? options.requestId : options.toolCallId
  const envelope = {
    type: 'permission',
    key: `${options.sessionId}:${keySuffix}:resolved`,
    headers: { operation: 'insert' },
    value: {
      kind: 'approval_resolved',
      sessionId: options.sessionId,
      ...('requestId' in options ? { requestId: options.requestId } : {}),
      ...('toolCallId' in options ? { toolCallId: options.toolCallId } : {}),
      allow: options.allow,
      resolvedBy: options.resolvedBy ?? '@fireline/client',
      createdAtMs: Date.now(),
    },
  }
  await stream.append(JSON.stringify(envelope), { contentType: 'application/json' })
}
