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
  AwakeableTimeoutError,
  AwakeableAlreadyResolvedError,
  rejectAwakeable,
  promptCompletionKey,
  raceAwakeables,
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
    streamMock.mockResolvedValueOnce({
      async json() {
        return []
      },
    })
    streamMock.mockResolvedValueOnce({
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

  it('ctx.awakeable reuses an already-resolved durable completion without appending a waiting row', async () => {
    streamMock.mockResolvedValueOnce({
      async json() {
        return [
          {
            type: 'awakeable',
            key: 'prompt:session-r:request-r:resolved',
            value: {
              kind: 'awakeable_resolved',
              sessionId: 'session-r',
              requestId: 'request-r',
              value: { approved: true },
              resolvedAtMs: 1,
            },
          },
        ]
      },
    })

    const ctx = workflowContext({
      stateStreamUrl: 'http://127.0.0.1:7474/v1/stream/state',
    })
    const key = promptCompletionKey({
      sessionId: 'session-r' as SessionId,
      requestId: 'request-r' as RequestId,
    })

    await expect(ctx.awakeable<{ approved: boolean }>(key).promise).resolves.toEqual({
      approved: true,
    })
    expect(appendMock).not.toHaveBeenCalled()
    expect(streamMock).toHaveBeenCalledTimes(1)
  })

  it('ctx.awakeable reuses an existing waiting row on replay instead of appending a duplicate', async () => {
    const unsubscribe = vi.fn()
    streamMock.mockResolvedValueOnce({
      async json() {
        return [
          {
            type: 'awakeable',
            key: 'prompt:session-p:request-p:waiting',
            value: {
              kind: 'awakeable_waiting',
              sessionId: 'session-p',
              requestId: 'request-p',
              createdAtMs: 1,
            },
          },
        ]
      },
    })
    streamMock.mockResolvedValueOnce({
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
              key: 'prompt:session-p:request-p:resolved',
              value: {
                kind: 'awakeable_resolved',
                sessionId: 'session-p',
                requestId: 'request-p',
                value: true,
                resolvedAtMs: 2,
              },
            },
          ],
          offset: '2',
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
      sessionId: 'session-p' as SessionId,
      requestId: 'request-p' as RequestId,
    })

    await expect(ctx.awakeable<boolean>(key).promise).resolves.toBe(true)
    expect(appendMock).not.toHaveBeenCalled()
    expect(unsubscribe).toHaveBeenCalledTimes(1)
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

  it('ctx.awakeable rejects on the matching canonical rejection envelope', async () => {
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
              key: 'prompt:session-a:request-a:rejected',
              value: {
                kind: 'awakeable_rejected',
                sessionId: 'session-a',
                requestId: 'request-a',
                error: { reason: 'policy denied' },
                rejectedAtMs: 1,
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

    await expect(ctx.awakeable<{ approved: boolean }>(key).promise).rejects.toThrow(
      'policy denied',
    )
    expect(unsubscribe).toHaveBeenCalledTimes(1)
  })

  it('rejectAwakeable writes W3C trace context onto the canonical rejection envelope', async () => {
    streamMock.mockResolvedValue({
      async json() {
        return []
      },
    })
    appendMock.mockResolvedValue()

    await rejectAwakeable({
      streamUrl: 'http://127.0.0.1:7474/v1/stream/state',
      key: promptCompletionKey({
        sessionId: 'session-a' as SessionId,
        requestId: 'request-a' as RequestId,
      }),
      error: { reason: 'policy denied' },
      traceContext: {
        traceparent: '00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01',
        tracestate: 'vendor=value',
        baggage: 'scope=review',
      },
    })

    const rejectedEnvelope = JSON.parse(String(appendMock.mock.calls[0]?.[0])) as {
      readonly type: string
      readonly key: string
      readonly value: {
        readonly kind: string
        readonly _meta?: Record<string, string>
      }
    }

    expect(rejectedEnvelope).toMatchObject({
      type: 'awakeable',
      key: 'prompt:session-a:request-a:rejected',
      value: {
        kind: 'awakeable_rejected',
        _meta: {
          traceparent:
            '00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01',
          tracestate: 'vendor=value',
          baggage: 'scope=review',
        },
      },
    })
  })

  it('raceAwakeables returns the first resolved branch and preserves winner trace context', async () => {
    appendMock.mockResolvedValue()
    const subscribers: Array<
      (
        batch: JsonBatch<{
          readonly type: string
          readonly key: string
          readonly value?: Record<string, unknown>
        }>,
      ) => void
    > = []
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
        subscribers.push(subscriber)
        return vi.fn()
      },
    })

    const ctx = workflowContext({
      stateStreamUrl: 'http://127.0.0.1:7474/v1/stream/state',
    })
    const first = ctx.awakeable<string>(
      promptCompletionKey({
        sessionId: 'session-a' as SessionId,
        requestId: 'request-a' as RequestId,
      }),
    )
    const second = ctx.awakeable<string>(
      promptCompletionKey({
        sessionId: 'session-a' as SessionId,
        requestId: 'request-b' as RequestId,
      }),
    )

    await Promise.resolve()
    await Promise.resolve()
    expect(subscribers).toHaveLength(2)

    const raced = raceAwakeables([first, second])

    subscribers[1]!({
      items: [
        {
          type: 'awakeable',
          key: 'prompt:session-a:request-b:resolved',
          value: {
            kind: 'awakeable_resolved',
            sessionId: 'session-a',
            requestId: 'request-b',
            value: 'winner-b',
            resolvedAtMs: 1,
            _meta: {
              traceparent:
                '00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01',
            },
          },
        },
      ],
      offset: '2',
      upToDate: true,
      streamClosed: false,
    })

    await expect(raced).resolves.toEqual({
      winnerIndex: 1,
      winnerKey: promptCompletionKey({
        sessionId: 'session-a' as SessionId,
        requestId: 'request-b' as RequestId,
      }),
      value: 'winner-b',
      traceContext: {
        traceparent:
          '00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01',
        tracestate: undefined,
        baggage: undefined,
      },
    })

    subscribers[0]!({
      items: [
        {
          type: 'awakeable',
          key: 'prompt:session-a:request-a:resolved',
          value: {
            kind: 'awakeable_resolved',
            sessionId: 'session-a',
            requestId: 'request-a',
            value: 'later-a',
            resolvedAtMs: 2,
          },
        },
      ],
      offset: '3',
      upToDate: true,
      streamClosed: false,
    })

    await expect(first.promise).resolves.toBe('later-a')
  })

  it('awakeable.withTimeout stays signature-only until DS Phase 6 lands', async () => {
    appendMock.mockResolvedValue()
    streamMock.mockResolvedValue({
      subscribeJson() {
        return vi.fn()
      },
    })

    const ctx = workflowContext({
      stateStreamUrl: 'http://127.0.0.1:7474/v1/stream/state',
    })
    const awakeable = ctx.awakeable<boolean>(
      promptCompletionKey({
        sessionId: 'session-timeout' as SessionId,
        requestId: 'request-timeout' as RequestId,
      }),
    )

    await expect(awakeable.withTimeout(30_000)).rejects.toBeInstanceOf(
      AwakeableTimeoutError,
    )
  })
})
