/**
 * Shared session-boundary data for the Host and Orchestrator primitives in
 * `docs/proposals/client-primitives.md`, aligned with the substrate mapping in
 * `docs/explorations/managed-agents-mapping.md`.
 */
import type { JsonValue, SuspendReasonSpec } from './combinator.js'

export type SessionHandle = { readonly id: string; readonly kind: string }

export type SessionStatus =
  | { readonly kind: 'created' }
  | { readonly kind: 'running' }
  | { readonly kind: 'idle' }
  | { readonly kind: 'needs_wake' }
  | { readonly kind: 'stopped' }
  | { readonly kind: 'error'; readonly message: string }

export type WakeOutcome =
  | { readonly kind: 'noop' }
  | { readonly kind: 'advanced'; readonly steps: number }
  | { readonly kind: 'blocked'; readonly reason: SuspendReasonSpec }

export type SessionInput =
  | { readonly kind: 'prompt'; readonly text: string }
  | {
      readonly kind: 'tool_result'
      readonly tool_call_id: string
      readonly result: JsonValue
    }

export type SessionOutput =
  | { readonly kind: 'message'; readonly message: JsonValue }
  | { readonly kind: 'chunk'; readonly chunk: JsonValue }
  | { readonly kind: 'tool_call'; readonly tool_call: JsonValue }
  | { readonly kind: 'done' }

export type Unsubscribe = () => void
