import { describe, expect, it } from 'vitest'

import { firelineState } from '../src/schema.js'

describe('firelineState schema', () => {
  it('defines the expected collections', () => {
    expect(Object.keys(firelineState).sort()).toEqual([
      'childSessionEdges',
      'chunks',
      'connections',
      'pendingRequests',
      'permissions',
      'promptTurns',
      'runtimeInstances',
      'sessions',
      'terminals',
    ])
  })

  it('creates a valid prompt turn insert event', () => {
    const event = firelineState.promptTurns.insert({
      value: {
        promptTurnId: 'turn-1',
        logicalConnectionId: 'conn-1',
        sessionId: 'session-1',
        requestId: 'req-1',
        text: 'hello',
        state: 'active',
        startedAt: 1,
      },
    })

    expect(event.type).toBe('prompt_turn')
    expect(event.key).toBe('turn-1')
    expect(event.headers.operation).toBe('insert')
  })

  it('creates a valid child-session edge insert event', () => {
    const event = firelineState.childSessionEdges.insert({
      value: {
        edgeId: 'edge-1',
        traceId: 'trace-1',
        parentRuntimeId: 'runtime-a',
        parentSessionId: 'session-a',
        parentPromptTurnId: 'turn-a',
        childRuntimeId: 'runtime-b',
        childSessionId: 'session-b',
        createdAt: 1,
      },
    })

    expect(event.type).toBe('child_session_edge')
    expect(event.key).toBe('edge-1')
    expect(event.headers.operation).toBe('insert')
  })
})
