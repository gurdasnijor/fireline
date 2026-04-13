import { describe, expect, it } from 'vitest'

import { firelineState } from '../src/schema.js'

describe('firelineState schema', () => {
  it('defines the expected collections', () => {
    expect(Object.keys(firelineState).sort()).toEqual([
      'chunks',
      'permissions',
      'promptRequests',
      'sessions',
    ])
  })

  it('creates a valid prompt_request insert event', () => {
    const event = firelineState.promptRequests.insert({
      key: 'session-1:req-1',
      value: {
        promptRequestKey: 'session-1:req-1',
        sessionId: 'session-1',
        requestId: 'req-1',
        text: 'hello',
        state: 'active',
        startedAt: 1,
      },
    })

    expect(event.type).toBe('prompt_request')
    expect(event.key).toBe('session-1:req-1')
    expect(event.headers.operation).toBe('insert')
  })

  it('creates a valid permission insert event', () => {
    const event = firelineState.permissions.insert({
      key: 'session-1:req-1',
      value: {
        permissionEventKey: 'session-1:req-1',
        kind: 'permission_request',
        sessionId: 'session-1',
        requestId: 'req-1',
        reason: 'approval required',
        createdAtMs: 1,
      },
    })

    expect(event.type).toBe('permission')
    expect(event.key).toBe('session-1:req-1')
    expect(event.headers.operation).toBe('insert')
  })

  it('creates a valid session_v2 insert event', () => {
    const event = firelineState.sessions.insert({
      value: {
        sessionId: 'session-1',
        state: 'active',
        supportsLoadSession: true,
        createdAt: 1,
        updatedAt: 1,
        lastSeenAt: 1,
      },
    })

    expect(event.type).toBe('session_v2')
    expect(event.key).toBe('session-1')
    expect(event.headers.operation).toBe('insert')
  })

  it('creates a valid chunk_v2 insert event', () => {
    const event = firelineState.chunks.insert({
      value: {
        chunkKey: 'chunk:sess-1:req-1:0',
        sessionId: 'session-1',
        requestId: 'req-1',
        update: {
          sessionUpdate: 'agent_message_chunk',
          content: { type: 'text', text: 'hello' },
        },
        createdAt: 1,
      },
    })

    expect(event.type).toBe('chunk_v2')
    expect(event.key).toBe('chunk:sess-1:req-1:0')
    expect(event.headers.operation).toBe('insert')
  })
})
