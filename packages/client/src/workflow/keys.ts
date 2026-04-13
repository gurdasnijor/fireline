import type { RequestId, SessionId, ToolCallId } from '../acp-ids.js'

/**
 * Canonical completion identity shared by durable subscribers and awakeables.
 *
 * This mirrors the Rust `CompletionKey` enum on the durable-subscriber
 * substrate. Fireline does not mint a separate imperative workflow id.
 *
 * @example `const key = promptCompletionKey({ sessionId, requestId })`
 *
 * @remarks Anthropic primitive: Session.
 */
export type CompletionKey =
  | {
      readonly kind: 'prompt'
      readonly sessionId: SessionId
      readonly requestId: RequestId
    }
  | {
      readonly kind: 'tool'
      readonly sessionId: SessionId
      readonly toolCallId: ToolCallId
    }
  | {
      readonly kind: 'session'
      readonly sessionId: SessionId
    }

/**
 * Awakeables reuse the canonical completion-key surface directly.
 *
 * This is an alias, not a second identifier family.
 *
 * @example `const key: AwakeableKey = sessionCompletionKey(sessionId)`
 *
 * @remarks Anthropic primitive: Session.
 */
export type AwakeableKey = CompletionKey

/**
 * W3C trace context copied onto durable awakeable completion envelopes.
 *
 * @example `const trace: WorkflowTraceContext = { traceparent }`
 *
 * @remarks Anthropic primitive: Session.
 */
export interface WorkflowTraceContext {
  readonly traceparent?: string
  readonly tracestate?: string
  readonly baggage?: string
}

/**
 * Builds a prompt-scoped canonical completion key.
 *
 * @example `const key = promptCompletionKey({ sessionId, requestId })`
 *
 * @remarks Anthropic primitive: Session.
 */
export function promptCompletionKey(input: {
  readonly sessionId: SessionId
  readonly requestId: RequestId
}): CompletionKey {
  return {
    kind: 'prompt',
    sessionId: input.sessionId,
    requestId: input.requestId,
  }
}

/**
 * Builds a tool-scoped canonical completion key.
 *
 * @example `const key = toolCompletionKey({ sessionId, toolCallId })`
 *
 * @remarks Anthropic primitive: Session.
 */
export function toolCompletionKey(input: {
  readonly sessionId: SessionId
  readonly toolCallId: ToolCallId
}): CompletionKey {
  return {
    kind: 'tool',
    sessionId: input.sessionId,
    toolCallId: input.toolCallId,
  }
}

/**
 * Builds a session-scoped canonical completion key.
 *
 * @example `const key = sessionCompletionKey(sessionId)`
 *
 * @remarks Anthropic primitive: Session.
 */
export function sessionCompletionKey(sessionId: SessionId): CompletionKey {
  return {
    kind: 'session',
    sessionId,
  }
}

export function completionKeyStorageKey(key: CompletionKey): string {
  switch (key.kind) {
    case 'prompt':
      return `prompt:${key.sessionId}:${requestIdStorageKey(key.requestId)}`
    case 'tool':
      return `tool:${key.sessionId}:${key.toolCallId}`
    case 'session':
      return `session:${key.sessionId}`
  }
}

function requestIdStorageKey(value: RequestId): string {
  return value === null ? 'null' : String(value)
}
