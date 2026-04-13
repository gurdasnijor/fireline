import type {
  RequestId,
  SessionId,
  SessionUpdate,
  StopReason,
  ToolCallId,
} from '@agentclientprotocol/sdk'

// Canonical ACP identifier types for the agent plane.
// These are thin re-exports of the ACP SDK. Fireline does NOT invent its own
// agent-identity types. See docs/proposals/acp-canonical-identifiers.md.
//
// ToolCallId note: repository scan on 2026-04-12 found no current Fireline
// tool-execution seam exposing canonical ToolCallId directly on the Rust side,
// so a future upstream ACP issue may still be needed. Phase 1 is additive only
// and does not block on that gap.
//
// The ACP SDK currently exposes these as structural string/JSON-RPC aliases,
// not nominal brands. Phase 1 intentionally does not add local branding.

export type { SessionId, RequestId, ToolCallId, SessionUpdate, StopReason }

export interface PromptRequestRef {
  readonly sessionId: import('@agentclientprotocol/sdk').SessionId
  readonly requestId: import('@agentclientprotocol/sdk').RequestId
}

export interface ToolInvocationRef {
  readonly sessionId: import('@agentclientprotocol/sdk').SessionId
  readonly toolCallId: import('@agentclientprotocol/sdk').ToolCallId
}

export function requestIdCollectionKey(requestId: RequestId): string | number {
  return requestId === null ? 'null' : requestId
}

export function promptRequestCollectionKey(
  sessionId: SessionId,
  requestId: RequestId,
): string {
  return `${sessionId}:${requestIdCollectionKey(requestId)}`
}
