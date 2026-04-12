import {
  approve as currentApprove,
  budget as currentBudget,
  contextInjection as currentContextInjection,
  peer as currentPeer,
  trace as currentTrace,
} from '../../packages/client/src/middleware.ts'
import type { ContextInjectionMiddleware } from '../../packages/client/src/types.ts'

export const trace = currentTrace
export const budget = currentBudget
export const peer = currentPeer

export function approve(options: {
  readonly scope: 'tool_calls' | 'all'
  readonly timeoutMs?: number
  readonly webhook?: string
}) {
  return currentApprove({ scope: options.scope, timeoutMs: options.timeoutMs })
}

export function contextInjection(options: {
  readonly prependText?: string
  readonly placement?: ContextInjectionMiddleware['placement']
  readonly files?: readonly string[]
  readonly sources?: ContextInjectionMiddleware['sources']
}) {
  return currentContextInjection({
    prependText: options.prependText,
    placement: options.placement,
    sources:
      options.sources ??
      options.files?.map((path) => ({ kind: 'workspaceFile', path }) as const),
  })
}
