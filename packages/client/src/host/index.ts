/**
 * Public Host primitive types and interface per the Anthropic managed-agent
 * taxonomy, as described in `docs/proposals/client-primitives.md` and
 * `docs/proposals/runtime-host-split.md` §7.
 *
 * A `Host` provisions agent runtime instances. Sessions live *inside* a
 * provisioned runtime and are minted by the ACP data plane (`session/new`
 * per https://agentclientprotocol.com/protocol/session-setup). The Host
 * primitive does not own session lifecycle — it owns runtime lifecycle and
 * returns an `acp` endpoint that clients use to open an ACP connection
 * directly.
 */
import type { JsonValue, SuspendReasonSpec } from '../core/combinator.js'
import type { CapabilityRef } from '../core/tool.js'
import type { ResourceRef } from '../core/resource.js'
import type { Topology } from '../core/combinator.js'

export type Endpoint = {
  readonly url: string
  readonly headers?: Readonly<Record<string, string>>
}

export type HostHandle = {
  readonly id: string
  readonly kind: string
  readonly acp: Endpoint
  readonly state: Endpoint
}

export type HostStatus =
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

export type ProvisionSpec = {
  readonly topology?: Topology
  readonly resources?: readonly ResourceRef[]
  readonly capabilities?: readonly CapabilityRef[]
  readonly agentCommand?: readonly string[]
  readonly model?: string
  readonly metadata?: Readonly<Record<string, JsonValue>>
}

export interface Host {
  provision(spec: ProvisionSpec): Promise<HostHandle>
  wake(handle: HostHandle): Promise<WakeOutcome>
  status(handle: HostHandle): Promise<HostStatus>
  stop(handle: HostHandle): Promise<void>
}
