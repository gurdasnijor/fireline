import { afterEach, describe, expect, expectTypeOf, it, vi } from 'vitest'
import type { JsonBatch } from '@durable-streams/client'
import type { RequestId, SessionId } from '@agentclientprotocol/sdk'

const { appendMock, streamMock } = vi.hoisted(() => ({
  appendMock: vi.fn<
    (data: string, options?: { readonly contentType?: string }) => Promise<void>
  >(),
  streamMock: vi.fn(),
}))

vi.mock('@durable-streams/client', () => {
  class MockDurableStream {
    constructor(_options: unknown) {}

    append(
      data: string,
      options?: { readonly contentType?: string },
    ): Promise<void> {
      return appendMock(data, options)
    }
  }

  return {
    DurableStream: MockDurableStream,
    stream: streamMock,
  }
})

import {
  AwakeableAlreadyResolvedError,
  promptCompletionKey,
  resolveAwakeable,
  workflowContext,
} from '../src/index.js'

describe('workflow awakeable surface', () => {
  afterEach(() => {
    appendMock.mockReset()
    streamMock.mockReset()
    vi.restoreAllMocks()
  })

  it('ctx.awakeable resolves on the matching canonical completion and preserves T inference', async () => {
    appendMock.mockResolvedValue()
    const unsubscribe = vi.fn()
    streamMock.mockResolvedValue({
      subscribeJson(
        subscriber: (
          batch: JsonBatch<{
            readonly type: string
            readonly key: string
            readonly value?: Record<string, unknown>
          }>,
        ) => void,
      ) {
        subscriber({
          items: [
            {
              type: 'awakeable',
              key: 'prompt:session-a:request-a:resolved',
              value: {
                kind: 'awakeable_resolved',
                sessionId: 'session-a',
                requestId: 'request-a',
                value: { approved: true },
                resolvedAtMs: 1,
              },
            },
          ],
          offset: '1',
          upToDate: true,
          streamClosed: false,
        })
        return unsubscribe
      },
    })

    const ctx = workflowContext({
      stateStreamUrl: 'http://127.0.0.1:7474/v1/stream/state',
    })
    const key = promptCompletionKey({
      sessionId: 'session-a' as SessionId,
      requestId: 'request-a' as RequestId,
    })

    const approval = ctx.awakeable<{ approved: boolean }>(key)

    expectTypeOf(approval.promise).toEqualTypeOf<Promise<{ approved: boolean }>>()
    await expect(approval.promise).resolves.toEqual({ approved: true })
    expect(unsubscribe).toHaveBeenCalledTimes(1)

    const waitingEnvelope = JSON.parse(String(appendMock.mock.calls[0]?.[0])) as {
      readonly type: string
      readonly key: string
      readonly value: Record<string, unknown>
    }
    expect(waitingEnvelope).toMatchObject({
      type: 'awakeable',
      key: 'prompt:session-a:request-a:waiting',
      value: {
        kind: 'awakeable_waiting',
        sessionId: 'session-a',
        requestId: 'request-a',
      },
    })
  })

  it('resolveAwakeable rejects duplicate completions for the same canonical key', async () => {
    streamMock.mockResolvedValue({
      async json() {
        return [
          {
            type: 'awakeable',
            key: 'prompt:session-a:request-a:resolved',
            value: { kind: 'awakeable_resolved' },
          },
        ]
      },
    })

    await expect(
      resolveAwakeable({
        streamUrl: 'http://127.0.0.1:7474/v1/stream/state',
        key: promptCompletionKey({
          sessionId: 'session-a' as SessionId,
          requestId: 'request-a' as RequestId,
        }),
        value: true,
      }),
    ).rejects.toBeInstanceOf(AwakeableAlreadyResolvedError)

    expect(appendMock).not.toHaveBeenCalled()
  })

  it('resolveAwakeable writes W3C trace context onto the canonical completion envelope', async () => {
    streamMock.mockResolvedValue({
      async json() {
        return []
      },
    })
    appendMock.mockResolvedValue()

    await resolveAwakeable({
      streamUrl: 'http://127.0.0.1:7474/v1/stream/state',
      key: promptCompletionKey({
        sessionId: 'session-a' as SessionId,
        requestId: 'request-a' as RequestId,
      }),
      value: { approved: true },
      traceContext: {
        traceparent: '00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01',
        tracestate: 'vendor=value',
        baggage: 'scope=review',
      },
    })

    const resolvedEnvelope = JSON.parse(String(appendMock.mock.calls[0]?.[0])) as {
      readonly type: string
      readonly key: string
      readonly value: {
        readonly kind: string
        readonly _meta?: Record<string, string>
      }
    }

    expect(resolvedEnvelope).toMatchObject({
      type: 'awakeable',
      key: 'prompt:session-a:request-a:resolved',
      value: {
        kind: 'awakeable_resolved',
        _meta: {
          traceparent:
            '00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01',
          tracestate: 'vendor=value',
          baggage: 'scope=review',
        },
      },
    })
  })
})
