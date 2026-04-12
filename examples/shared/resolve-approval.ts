import { DurableStream } from '@durable-streams/client'

export async function resolveApproval(
  streamUrl: string,
  sessionId: string,
  requestId: string,
  allow: boolean,
) {
  const stream = new DurableStream({ url: streamUrl })
  const value = { kind: 'approval_resolved', sessionId, requestId, allow, resolvedBy: 'approval-workflow', createdAtMs: Date.now() }
  await stream.append(JSON.stringify({ type: 'permission', key: `${sessionId}:${requestId}:resolved`, headers: { operation: 'insert' }, value }), { contentType: 'application/json' })
}
