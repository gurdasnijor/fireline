/**
 * Session request values for the managed-agent Host primitive from
 * `docs/proposals/client-primitives.md`, composed from the substrate refs in
 * `docs/explorations/managed-agents-mapping.md`.
 */
import type { JsonValue, Topology } from './combinator.js'
import type { ResourceRef } from './resource.js'
import type { CapabilityRef } from './tool.js'

export type SessionSpec = {
  readonly topology?: Topology
  readonly resources?: readonly ResourceRef[]
  readonly capabilities?: readonly CapabilityRef[]
  readonly agentCommand?: readonly string[]
  readonly model?: string
  readonly initialPrompt?: string
  readonly metadata?: Readonly<Record<string, JsonValue>>
}
