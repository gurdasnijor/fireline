import type {
  ApproveMiddleware,
  BudgetMiddleware,
  ContextInjectionMiddleware,
  ContextSourceSpec,
  PeerMiddleware,
  TraceMiddleware,
} from './types.js'

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

/**
 * Builds an approval middleware spec for prompt- or tool-scoped approval gates.
 *
 * @example `const mw = approve({ scope: 'tool_calls', timeoutMs: 60_000 })`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export function approve(options: {
  readonly scope: 'tool_calls' | 'all'
  readonly timeoutMs?: number
}): ApproveMiddleware {
  return {
    kind: 'approve',
    ...cloneDefined(options),
  }
}

/**
 * Builds a budget middleware spec that caps harness token usage.
 *
 * @example `const mw = budget({ tokens: 100_000 })`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export function budget(options: {
  readonly tokens?: number
} = {}): BudgetMiddleware {
  return {
    kind: 'budget',
    ...cloneDefined(options),
  }
}

/**
 * Builds a context-injection middleware spec for preloading prompt context.
 *
 * @example `const mw = contextInjection({ prependText: 'Repository policy' })`
 *
 * @remarks Anthropic primitive: Middleware.
 */
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

/**
 * Builds a context-injection middleware spec from a source list.
 *
 * @example `const mw = inject([{ kind: 'workspaceFile', path: '/workspace/README.md' }])`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export function inject(
  sources: readonly ContextSourceSpec[],
  options: {
    readonly placement?: ContextInjectionMiddleware['placement']
    readonly prependText?: string
  } = {},
): ContextInjectionMiddleware {
  return contextInjection({
    ...options,
    sources,
  })
}

/**
 * Builds a peer middleware spec that enables the `peer_mcp` topology component.
 *
 * @example `const mw = peer()`
 *
 * @remarks Anthropic primitive: Middleware.
 */
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
