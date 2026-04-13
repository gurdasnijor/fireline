import { afterEach, describe, expect, expectTypeOf, it, vi } from 'vitest'

import { agent, compose, middleware, sandbox, Sandbox } from '../src/sandbox.js'
import { autoApprove, durableSubscriber, telegram, webhook } from '../src/middleware.js'

describe('durable-subscriber middleware surface', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  it('serializes webhook, telegram, and auto-approve middleware into topology components', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          id: 'sandbox-1',
          provider: 'local',
          acp: { url: 'ws://127.0.0.1:9000' },
          state: { url: 'http://127.0.0.1:7474/v1/stream/state' },
        }),
        {
          status: 200,
          headers: { 'content-type': 'application/json' },
        },
      ),
    )
    vi.stubGlobal('fetch', fetchMock)

    const harness = compose(
      sandbox({ provider: 'local' }),
      middleware([
        webhook({
          target: 'slack-approvals',
          url: 'https://hooks.slack.com/services/demo',
          events: ['permission_request'],
          keyBy: 'session_request',
        }),
        telegram({
          token: { ref: 'secret:telegram-bot' },
          chatId: '1234',
        }),
        autoApprove(),
      ]),
      agent(['node', 'agent.mjs']),
    ).as('reviewer')

    await new Sandbox({ serverUrl: 'http://127.0.0.1:4440' }).provision(harness)

    expect(fetchMock).toHaveBeenCalledTimes(1)
    const [, request] = fetchMock.mock.calls[0] as [string, RequestInit]
    const body = JSON.parse(String(request.body)) as {
      readonly topology: {
        readonly components: readonly Array<{
          readonly name: string
          readonly config?: Record<string, unknown>
        }>
      }
    }

    expect(body.topology.components).toEqual([
      {
        name: 'webhook_subscriber',
        config: {
          target: 'slack-approvals',
          events: [{ kind: 'permission_request' }],
          targetConfig: {
            url: 'https://hooks.slack.com/services/demo',
            headers: {},
            timeoutMs: 5_000,
            maxAttempts: 1,
            cursorStream: 'subscribers:webhook:slack-approvals',
            deadLetterStream: 'subscribers:webhook:slack-approvals:dead-letter',
          },
        },
      },
      {
        name: 'telegram_subscriber',
        config: {
          token: { ref: 'secret:telegram-bot' },
          chatId: '1234',
          events: ['permission_request'],
          keyBy: 'session_request',
        },
      },
      {
        name: 'auto_approve',
      },
    ])
  })

  it('keeps durable-subscriber key strategies canonical at the type surface', () => {
    const profile = durableSubscriber(
      webhook({
        url: 'https://example.com/hooks/demo',
        events: ['permission_request'],
        keyBy: 'session_request',
      }),
    )

    expect(profile.kind).toBe('webhook')
    expectTypeOf(profile.keyBy).toEqualTypeOf<
      'session_request' | 'session_tool_call' | undefined
    >()
  })

  it('requires a telegram target or token', () => {
    expect(() => telegram({})).toThrowError(
      'telegram middleware requires either target or token',
    )
  })

  it('requires a concrete webhook url for live lowering', () => {
    expect(() =>
      webhook({
        target: 'slack-approvals',
        events: ['permission_request'],
      }),
    ).toThrowError(
      'webhook middleware currently requires url for live lowering; target-only routing is pending host target config support',
    )
  })
})
