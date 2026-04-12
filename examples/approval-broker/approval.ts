// Third-party
import { DurableStream } from '@durable-streams/client'

export async function appendApprovalResolved(
  stateStreamUrl: string,
  sessionId: string,
  requestId: string,
  allow: boolean,
) {
  const stream = new DurableStream({ url: stateStreamUrl, contentType: 'application/json' })
  await stream.append(
    JSON.stringify({
      type: 'permission',
      key: `${sessionId}:${requestId}:resolved`,
      headers: { operation: 'insert' },
      value: {
        kind: 'approval_resolved',
        sessionId,
        requestId,
        allow,
        resolvedBy: 'examples-approval-broker',
        createdAtMs: Date.now(),
      },
    }),
  )
}
