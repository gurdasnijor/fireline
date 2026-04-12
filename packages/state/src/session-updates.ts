import type { SessionUpdate } from './acp-types.js'

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null
    ? (value as Record<string, unknown>)
    : null
}

function textContent(update: SessionUpdate): string {
  const record = asRecord(update)
  const content = asRecord(record?.content)
  return typeof content?.text === 'string' ? content.text : ''
}

export function sessionUpdateKind(update: SessionUpdate): string {
  const record = asRecord(update)
  return typeof record?.sessionUpdate === 'string' ? record.sessionUpdate : ''
}

export function isToolCallSessionUpdate(update: SessionUpdate): boolean {
  const kind = sessionUpdateKind(update)
  return kind === 'tool_call' || kind === 'tool_call_update'
}

export function sessionUpdateToolCallId(
  update: SessionUpdate,
): string | undefined {
  const record = asRecord(update)
  return typeof record?.toolCallId === 'string' ? record.toolCallId : undefined
}

export function sessionUpdateStatus(update: SessionUpdate): string | undefined {
  const record = asRecord(update)
  return typeof record?.status === 'string' ? record.status : undefined
}

export function sessionUpdateTitle(update: SessionUpdate): string | undefined {
  const record = asRecord(update)
  if (typeof record?.title === 'string') {
    return record.title
  }
  return typeof record?.toolName === 'string' ? record.toolName : undefined
}

export function extractChunkTextPreview(update: SessionUpdate): string {
  switch (sessionUpdateKind(update)) {
    case 'user_message_chunk':
    case 'agent_message_chunk':
    case 'agent_thought_chunk':
      return textContent(update)
    case 'tool_call':
      return sessionUpdateTitle(update) ?? ''
    case 'tool_call_update':
      return sessionUpdateStatus(update) ?? ''
    default:
      return ''
  }
}
