/**
 * Public Sandbox primitive types for tool execution, aligned with the
 * Host/Sandbox/Orchestrator taxonomy in
 * `docs/proposals/runtime-host-split.md` §7 and the Anthropic primitive
 * source-of-truth in `docs/explorations/managed-agents-mapping.md`.
 *
 * Sandbox is deliberately separate from Host: Host runs a session's
 * agent, Sandbox runs one tool call at a time in an isolated executor.
 * The TLA model in `verification/spec/managed_agents.tla` encodes the
 * same split (sandboxIndex / SandboxProvision / SandboxExecute /
 * SandboxStop).
 */
import type { JsonValue } from '../core/index.js'
import type { CapabilityRef } from '../core/tool.js'

export type { CapabilityRef } from '../core/tool.js'

export type SandboxHandle = {
  readonly id: string
  readonly kind: string
}

export type SandboxSpec = {
  readonly runtime_key: string
  readonly capabilities?: readonly CapabilityRef[]
  readonly mount_paths?: readonly string[]
  readonly metadata?: Readonly<Record<string, JsonValue>>
}

export type ToolCall = {
  readonly tool_name: string
  readonly arguments: JsonValue
  readonly call_id?: string
}

export type ToolResult =
  | { readonly kind: 'ok'; readonly value: JsonValue }
  | { readonly kind: 'error'; readonly message: string }

export type SandboxStatus =
  | { readonly kind: 'provisioning' }
  | { readonly kind: 'ready' }
  | { readonly kind: 'executing'; readonly call_id: string }
  | { readonly kind: 'stopped' }
  | { readonly kind: 'error'; readonly message: string }

export interface Sandbox {
  provision(spec: SandboxSpec): Promise<SandboxHandle>
  execute(handle: SandboxHandle, call: ToolCall): Promise<ToolResult>
  status(handle: SandboxHandle): Promise<SandboxStatus>
  stop(handle: SandboxHandle): Promise<void>
}
