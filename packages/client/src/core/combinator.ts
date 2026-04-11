/**
 * Pure combinator data and helper constructors for the managed-agent substrate
 * described in `docs/proposals/client-primitives.md` and anchored to
 * `docs/explorations/managed-agents-mapping.md`.
 */
import type { CapabilityRef } from './tool.js'

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue }

export type JsonSchema = { readonly [key: string]: JsonValue }

export type Endpoint = {
  readonly url: string
  readonly headers?: Readonly<Record<string, string>>
}

export type ContextSourceRef =
  | { readonly kind: 'static_text'; readonly text: string }
  | { readonly kind: 'workspace_file'; readonly path: string }
  | { readonly kind: 'datetime' }

export type EffectPattern =
  | { readonly kind: 'any' }
  | { readonly kind: 'prompt_contains'; readonly needle: string }
  | { readonly kind: 'prompt_matches'; readonly regex: string; readonly flags?: string }
  | { readonly kind: 'tool_call'; readonly name?: string; readonly name_prefix?: string }
  | { readonly kind: 'peer_call' }
  | { readonly kind: 'any_of'; readonly patterns: readonly EffectPattern[] }

export type RewriteSpec =
  | { readonly kind: 'prepend_context'; readonly sources: readonly ContextSourceRef[] }
  | { readonly kind: 'route_to_peer'; readonly peer: string }
  | { readonly kind: 'replace_tool'; readonly from: string; readonly to: CapabilityRef }
  | { readonly kind: 'text_substitute'; readonly from: string; readonly to: string }

export type ProjectSpec =
  | { readonly kind: 'audit_effect' }
  | { readonly kind: 'durable_trace' }
  | { readonly kind: 'custom'; readonly entity_type: string }

export type SuspendReasonSpec =
  | {
      readonly kind: 'require_approval'
      readonly scope: 'tool_calls' | 'all' | 'matching'
      readonly matcher?: EffectPattern
      readonly timeout_ms?: number
    }
  | { readonly kind: 'require_budget_refresh' }
  | { readonly kind: 'wait_for_peer'; readonly peer: string }

export type ObserveSinkRef =
  | { readonly kind: 'state_stream'; readonly entity_type: string }
  | { readonly kind: 'metrics'; readonly name: string }

export type FanoutSplitSpec = { readonly kind: 'by_peer_list'; readonly peers: readonly string[] }
export type FanoutMergeSpec = { readonly kind: 'first_success' } | { readonly kind: 'all' }

export type Combinator =
  | { readonly kind: 'observe'; readonly sink: ObserveSinkRef }
  | { readonly kind: 'map_effect'; readonly rewrite: RewriteSpec; readonly when?: EffectPattern }
  | { readonly kind: 'append_to_session'; readonly project: ProjectSpec; readonly when?: EffectPattern }
  | { readonly kind: 'filter'; readonly when: EffectPattern; readonly reject: JsonValue }
  | { readonly kind: 'substitute'; readonly rewrite: RewriteSpec; readonly when: EffectPattern }
  | { readonly kind: 'suspend'; readonly reason: SuspendReasonSpec }
  | { readonly kind: 'fanout'; readonly split: FanoutSplitSpec; readonly merge: FanoutMergeSpec }

export type Topology = readonly Combinator[]

export const observe = (sink: ObserveSinkRef): Combinator => ({ kind: 'observe', sink })

export const audit = (): Combinator => ({
  kind: 'append_to_session',
  project: { kind: 'audit_effect' },
})

export const durableTrace = (): Combinator => ({
  kind: 'append_to_session',
  project: { kind: 'durable_trace' },
})

export const contextInjection = (sources: readonly ContextSourceRef[]): Combinator => ({
  kind: 'map_effect',
  rewrite: { kind: 'prepend_context', sources },
})

export const budget = (tokens: number): Combinator => ({
  kind: 'filter',
  when: { kind: 'any' },
  reject: { error: 'budget_exceeded', max_tokens: tokens },
})

export const approvalGate = (opts: {
  readonly scope: 'tool_calls' | 'all'
  readonly timeoutMs?: number
}): Combinator => ({
  kind: 'suspend',
  reason: { kind: 'require_approval', scope: opts.scope, timeout_ms: opts.timeoutMs },
})

export const approvalGateOnPattern = (opts: {
  readonly matcher: EffectPattern
  readonly timeoutMs?: number
}): Combinator => ({
  kind: 'suspend',
  reason: {
    kind: 'require_approval',
    scope: 'matching',
    matcher: opts.matcher,
    timeout_ms: opts.timeoutMs,
  },
})

export const peer = (peers: readonly string[]): Combinator => ({
  kind: 'substitute',
  rewrite: { kind: 'route_to_peer', peer: peers[0] /* simplification */ },
  when: { kind: 'peer_call' },
})

export const parallelPeers = (peers: readonly string[]): Combinator => ({
  kind: 'fanout',
  split: { kind: 'by_peer_list', peers },
  merge: { kind: 'first_success' },
})

export const topology = (...parts: readonly Combinator[]): Topology => parts
