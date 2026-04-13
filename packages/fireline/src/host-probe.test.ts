import assert from 'node:assert/strict'
import test from 'node:test'

import { probeExistingHostForRepl } from './host-probe.js'

test('probeExistingHostForRepl returns attachable host with latest session', async () => {
  const result = await probeExistingHostForRepl('http://127.0.0.1:4440', {
    healthCheck: async () => true,
    listSandboxes: async () => [
        {
          id: 'sandbox-1',
          provider: 'local',
          status: 'ready',
          acp: { url: 'ws://127.0.0.1:4440/acp' },
          state: { url: 'http://127.0.0.1:7474/v1/stream/state' },
          labels: {},
          createdAtMs: 1,
          updatedAtMs: 10,
        },
      ],
    loadDb: async () =>
      ({
        collections: {
          sessions: {
            toArray: [
              {
                sessionId: 'session-older',
                state: 'active',
                supportsLoadSession: true,
                createdAt: 1,
                updatedAt: 2,
                lastSeenAt: 3,
              },
              {
                sessionId: 'session-latest',
                state: 'active',
                supportsLoadSession: true,
                createdAt: 4,
                updatedAt: 5,
                lastSeenAt: 6,
              },
            ],
          },
        },
        close() {},
      }) as Awaited<ReturnType<typeof import('@fireline/client').db>>,
  })

  assert.deepEqual(result, {
    kind: 'attachable',
    handle: {
      id: 'sandbox-1',
      provider: 'local',
      acp: { url: 'ws://127.0.0.1:4440/acp' },
      state: { url: 'http://127.0.0.1:7474/v1/stream/state' },
    },
    latestSessionId: 'session-latest',
  })
})

test('probeExistingHostForRepl rejects ambiguous hosts', async () => {
  const result = await probeExistingHostForRepl('http://127.0.0.1:4440', {
    healthCheck: async () => true,
    listSandboxes: async () => [
        {
          id: 'sandbox-1',
          provider: 'local',
          status: 'ready',
          acp: { url: 'ws://127.0.0.1:4440/acp' },
          state: { url: 'http://127.0.0.1:7474/v1/stream/state-1' },
          labels: {},
          createdAtMs: 1,
          updatedAtMs: 10,
        },
        {
          id: 'sandbox-2',
          provider: 'local',
          status: 'idle',
          acp: { url: 'ws://127.0.0.1:4441/acp' },
          state: { url: 'http://127.0.0.1:7474/v1/stream/state-2' },
          labels: {},
          createdAtMs: 2,
          updatedAtMs: 20,
        },
      ],
  })

  assert.deepEqual(result, {
    kind: 'multiple-live-sandboxes',
    count: 2,
  })
})

test('probeExistingHostForRepl treats list failure as non-Fireline', async () => {
  const result = await probeExistingHostForRepl('http://127.0.0.1:4440', {
    healthCheck: async () => true,
    listSandboxes: async () => {
      throw new Error('not json')
    },
  })

  assert.deepEqual(result, { kind: 'not-fireline' })
})
