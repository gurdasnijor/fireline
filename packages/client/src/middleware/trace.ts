import type { TraceMiddleware } from '../types.js'

import { cloneDefined } from './shared.js'

/**
 * Builds a trace middleware spec that emits ACP traffic into a durable audit stream.
 *
 * @example `const mw = trace({ streamName: 'audit:demo' })`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export function trace(options: {
  readonly streamName?: string
  readonly includeMethods?: readonly string[]
} = {}): TraceMiddleware {
  return {
    kind: 'trace',
    ...cloneDefined(options),
  }
}
