import { describe, expect, it } from 'vitest'

import type { ChunkRow, PermissionRow, PromptTurnRow } from '../src/schema.js'
import { buildSessionLogs } from '../../../examples/flamecast-client/ui/lib/build-session-logs.js'

function promptTurn(
  overrides: Partial<PromptTurnRow> & Pick<PromptTurnRow, 'promptTurnId'>,
): PromptTurnRow {
  return {
    promptTurnId: overrides.promptTurnId,
    logicalConnectionId: overrides.logicalConnectionId ?? 'conn-1',
    sessionId: overrides.sessionId ?? 'session-1',
    requestId: overrides.requestId ?? 'req-1',
    text: overrides.text,
    state: overrides.state ?? 'completed',
    startedAt: overrides.startedAt ?? 1,
    completedAt: overrides.completedAt ?? 4,
    stopReason: overrides.stopReason ?? 'end_turn',
    position: overrides.position,
    traceId: overrides.traceId,
    parentPromptTurnId: overrides.parentPromptTurnId,
  }
}

function chunk(
  overrides: Partial<ChunkRow> & Pick<ChunkRow, 'chunkKey' | 'update'>,
): ChunkRow {
  return {
    chunkKey: overrides.chunkKey,
    sessionId: overrides.sessionId ?? 'session-1',
    requestId: overrides.requestId ?? 'req-1',
    toolCallId: overrides.toolCallId,
    update: overrides.update,
    createdAt: overrides.createdAt ?? 2,
  }
}

describe('Flamecast buildSessionLogs', () => {
  it('preserves tool call transcript updates for canonical SessionUpdate chunks', () => {
    const turns: PromptTurnRow[] = [
      promptTurn({
        promptTurnId: 'turn-1',
        requestId: 'req-1',
        text: 'Inspect README',
      }),
    ]
    const chunks: ChunkRow[] = [
      chunk({
        chunkKey: 'chunk-1',
        requestId: 'req-1',
        toolCallId: 'tc-1',
        createdAt: 2,
        update: {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc-1',
          title: 'Read README.md',
          status: 'pending',
        } as ChunkRow['update'],
      }),
      chunk({
        chunkKey: 'chunk-2',
        requestId: 'req-1',
        toolCallId: 'tc-1',
        createdAt: 3,
        update: {
          sessionUpdate: 'tool_call_update',
          toolCallId: 'tc-1',
          status: 'completed',
        } as ChunkRow['update'],
      }),
    ]
    const permissions: PermissionRow[] = []

    const logs = buildSessionLogs(turns, chunks, permissions)

    expect(logs).toEqual([
      {
        timestamp: new Date(1).toISOString(),
        type: 'prompt_sent',
        data: { text: 'Inspect README' },
      },
      {
        timestamp: new Date(2).toISOString(),
        type: 'session_update',
        data: {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc-1',
          title: 'Read README.md',
          status: 'pending',
        },
      },
      {
        timestamp: new Date(3).toISOString(),
        type: 'session_update',
        data: {
          sessionUpdate: 'tool_call_update',
          toolCallId: 'tc-1',
          status: 'completed',
        },
      },
      {
        timestamp: new Date(4).toISOString(),
        type: 'prompt_completed',
        data: { promptTurnId: 'turn-1', stopReason: 'end_turn' },
      },
    ])
  })
})
