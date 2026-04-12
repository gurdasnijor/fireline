import type {
  PromptRequestRef,
  RequestId,
  SessionId,
  ToolCallId,
  ToolInvocationRef,
} from './index.js'

export type CanonicalAcpTypeExportSmoke = {
  readonly sessionId: SessionId
  readonly requestId: RequestId
  readonly toolCallId: ToolCallId
  readonly prompt: PromptRequestRef
  readonly tool: ToolInvocationRef
}
