import type {
  ApproveMiddleware,
  BudgetMiddleware,
  ContextInjectionMiddleware,
  PeerMiddleware,
  TraceMiddleware,
} from './types.js'

export function trace(options: {
  readonly streamName?: string
  readonly includeMethods?: readonly string[]
} = {}): TraceMiddleware {
  return {
    kind: 'trace',
    ...cloneDefined(options),
  }
}

export function approve(options: {
  readonly scope: 'tool_calls' | 'all'
  readonly timeoutMs?: number
}): ApproveMiddleware {
  return {
    kind: 'approve',
    ...cloneDefined(options),
  }
}

export function budget(options: {
  readonly tokens?: number
} = {}): BudgetMiddleware {
  return {
    kind: 'budget',
    ...cloneDefined(options),
  }
}

export function contextInjection(options: {
  readonly prependText?: string
  readonly placement?: ContextInjectionMiddleware['placement']
  readonly sources?: ContextInjectionMiddleware['sources']
} = {}): ContextInjectionMiddleware {
  return {
    kind: 'contextInjection',
    ...cloneDefined(options),
  }
}

export function peer(options: {
  readonly peers?: readonly string[]
} = {}): PeerMiddleware {
  return {
    kind: 'peer',
    ...cloneDefined(options),
  }
}

function cloneDefined<T extends object>(value: T): T {
  return Object.fromEntries(
    Object.entries(value).filter(([, entry]) => entry !== undefined),
  ) as T
}
