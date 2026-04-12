import type { ResourceRef } from './core/index.js'
import type { ContextPlacement, ContextSourceSpec } from './topology.js'

export interface Endpoint {
  readonly url: string
  readonly headers?: Readonly<Record<string, string>>
}

export type SandboxStatus = 'creating' | 'ready' | 'busy' | 'idle' | 'stopped' | 'broken'

export interface AgentConfig {
  readonly kind: 'agent'
  readonly command: readonly string[]
}

export interface SandboxDefinition {
  readonly kind: 'sandbox'
  readonly resources?: readonly ResourceRef[]
  readonly envVars?: Readonly<Record<string, string>>
  readonly image?: string
  readonly provider?: string
  readonly labels?: Readonly<Record<string, string>>
}

export interface TraceMiddleware {
  readonly kind: 'trace'
  readonly streamName?: string
  readonly includeMethods?: readonly string[]
}

export interface ApproveMiddleware {
  readonly kind: 'approve'
  readonly scope: 'tool_calls' | 'all'
  readonly timeoutMs?: number
}

export interface BudgetMiddleware {
  readonly kind: 'budget'
  readonly tokens?: number
}

export interface ContextInjectionMiddleware {
  readonly kind: 'contextInjection'
  readonly prependText?: string
  readonly placement?: ContextPlacement
  readonly sources?: readonly ContextSourceSpec[]
}

export interface PeerMiddleware {
  readonly kind: 'peer'
  readonly peers?: readonly string[]
}

export type Middleware =
  | TraceMiddleware
  | ApproveMiddleware
  | BudgetMiddleware
  | ContextInjectionMiddleware
  | PeerMiddleware

export interface HarnessConfig<Name extends string = string> {
  readonly kind: 'harness'
  readonly name: Name
  readonly sandbox: SandboxDefinition
  readonly middleware: readonly Middleware[]
  readonly agent: AgentConfig
  readonly stateStream?: string
}

export type SandboxConfig<Name extends string = string> = HarnessConfig<Name>

export interface SandboxHandle {
  readonly id: string
  readonly provider: string
  readonly acp: Endpoint
  readonly state: Endpoint
}

export interface SandboxDescriptor extends SandboxHandle {
  readonly status: SandboxStatus
  readonly labels: Readonly<Record<string, string>>
  readonly createdAtMs: number
  readonly updatedAtMs: number
}
