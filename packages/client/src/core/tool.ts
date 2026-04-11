/**
 * Tool and capability descriptors for the managed-agent Tools primitive in
 * `docs/proposals/client-primitives.md`, aligned with
 * `docs/explorations/managed-agents-mapping.md`.
 */
import type { JsonSchema } from './combinator.js'

export type ToolDescriptor = {
  readonly name: string
  readonly description: string
  readonly input_schema: JsonSchema
}

export type TransportRef =
  | { readonly kind: 'mcp_url'; readonly url: string }
  | { readonly kind: 'peer_runtime'; readonly runtime_key: string }
  | { readonly kind: 'smithery'; readonly catalog: string; readonly tool: string }
  | { readonly kind: 'in_process'; readonly component_name: string }

export type CredentialRef =
  | { readonly kind: 'env'; readonly var: string }
  | { readonly kind: 'secret'; readonly key: string }
  | { readonly kind: 'oauth_token'; readonly provider: string; readonly account?: string }

export type CapabilityRef = {
  readonly descriptor: ToolDescriptor
  readonly transport_ref: TransportRef
  readonly credential_ref?: CredentialRef
}
