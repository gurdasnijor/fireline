import type { ResourceRef } from './resources.js'

export interface TopologyComponentSpec {
  readonly name: string
  readonly config?: Record<string, unknown>
}

export interface TopologySpec {
  readonly components: readonly TopologyComponentSpec[]
}

export type ContextPlacement = 'prepend' | 'append'

export type ContextSourceSpec =
  | { readonly kind: 'datetime' }
  | { readonly kind: 'workspaceFile'; readonly path: string }
  | { readonly kind: 'staticText'; readonly text: string }

/**
 * Connection details for an ACP or durable-state endpoint exposed by a sandbox.
 *
 * @example `console.log(handle.acp.url)`
 *
 * @remarks Anthropic primitive: Sandbox.
 */
export interface Endpoint {
  /** Absolute URL for the advertised endpoint. */
  readonly url: string
  /** Optional static headers required when connecting to the endpoint. */
  readonly headers?: Readonly<Record<string, string>>
}

/**
 * Lifecycle state reported for a sandbox descriptor.
 *
 * @example `if (descriptor.status === 'ready') await connect(handle.acp.url)`
 *
 * @remarks Anthropic primitive: Sandbox.
 */
export type SandboxStatus = 'creating' | 'ready' | 'busy' | 'idle' | 'stopped' | 'broken'

/**
 * Serializable agent process definition used inside a composed harness.
 *
 * @example `const cfg: AgentConfig = { kind: 'agent', command: ['npx', '-y', '@anthropic-ai/claude-code-acp'] }`
 *
 * @remarks Anthropic primitive: Harness.
 */
export interface AgentConfig {
  /** Stable discriminator for serialized agent configs. */
  readonly kind: 'agent'
  /** Command and arguments used to launch the ACP-speaking agent process. */
  readonly command: readonly string[]
}

/**
 * Serializable sandbox recipe that describes the execution environment.
 *
 * @example `const cfg: SandboxDefinition = { kind: 'sandbox', resources: [] }`
 *
 * @remarks Anthropic primitive: Sandbox.
 */
export interface SandboxDefinition {
  /** Stable discriminator for serialized sandbox definitions. */
  readonly kind: 'sandbox'
  /** Resources mounted into the sandbox before the agent starts. */
  readonly resources?: readonly ResourceRef[]
  /** Environment variables that the provider may inject into the sandbox. */
  readonly envVars?: Readonly<Record<string, string>>
  /** Optional provider-specific image identifier. */
  readonly image?: string
  /** Optional provider hint such as `local` or `docker`. */
  readonly provider?: string
  /** Labels used for lookup, routing, or fleet bookkeeping. */
  readonly labels?: Readonly<Record<string, string>>
}

/**
 * Middleware spec that enables durable tracing for ACP traffic.
 *
 * @example `const mw: TraceMiddleware = { kind: 'trace', streamName: 'audit:demo' }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface TraceMiddleware {
  /** Stable discriminator for trace middleware. */
  readonly kind: 'trace'
  /** Optional audit stream name; defaults to a generated value. */
  readonly streamName?: string
  /** Optional ACP methods to include in the audit stream. */
  readonly includeMethods?: readonly string[]
}

/**
 * Middleware spec that inserts approval gates into the ACP pipeline.
 *
 * @example `const mw: ApproveMiddleware = { kind: 'approve', scope: 'tool_calls' }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface ApproveMiddleware {
  /** Stable discriminator for approval middleware. */
  readonly kind: 'approve'
  /** Approval scope applied by the harness topology. */
  readonly scope: 'tool_calls' | 'all'
  /** Optional timeout for outstanding approvals. */
  readonly timeoutMs?: number
}

/**
 * Middleware spec that enforces token budgets inside the harness.
 *
 * @example `const mw: BudgetMiddleware = { kind: 'budget', tokens: 50_000 }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface BudgetMiddleware {
  /** Stable discriminator for budget middleware. */
  readonly kind: 'budget'
  /** Optional maximum token budget for the run. */
  readonly tokens?: number
}

/**
 * Middleware spec that injects additional context ahead of ACP prompts.
 *
 * @example `const mw: ContextInjectionMiddleware = { kind: 'contextInjection', prependText: 'Repo policy' }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface ContextInjectionMiddleware {
  /** Stable discriminator for context-injection middleware. */
  readonly kind: 'contextInjection'
  /** Optional static prefix inserted into the prompt context. */
  readonly prependText?: string
  /** Whether gathered context is prepended or appended. */
  readonly placement?: ContextPlacement
  /** Optional list of dynamic context sources. */
  readonly sources?: readonly ContextSourceSpec[]
}

/**
 * Middleware spec that enables peer MCP wiring for the harness topology.
 *
 * @example `const mw: PeerMiddleware = { kind: 'peer' }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface PeerMiddleware {
  /** Stable discriminator for peer middleware. */
  readonly kind: 'peer'
  /** Optional logical peer names reserved for later topology expansion. */
  readonly peers?: readonly string[]
}

/**
 * A single secret binding that maps a logical name to a credential reference
 * and an optional domain allow-list for outbound injection.
 *
 * @example `{ ref: 'secret:gh-pat', allow: 'api.github.com' }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface SecretBinding {
  /** Credential reference resolved by the host's credential resolver. */
  readonly ref: string
  /** Optional domain allow-list — the secret is only injected for requests to these domains. */
  readonly allow?: string | readonly string[]
}

/**
 * Middleware spec that injects credentials at call time without exposing
 * plaintext to the agent.
 *
 * @example `const mw: SecretsProxyMiddleware = { kind: 'secretsProxy', bindings: { GITHUB_TOKEN: { ref: 'secret:gh-pat', allow: 'api.github.com' } } }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface SecretsProxyMiddleware {
  /** Stable discriminator for secrets proxy middleware. */
  readonly kind: 'secretsProxy'
  /** Map from logical secret name to credential binding. */
  readonly bindings: Readonly<Record<string, SecretBinding>>
}

/**
 * Union of every serializable middleware spec accepted by `compose()`.
 *
 * @example `const chain: Middleware[] = [trace(), budget({ tokens: 20_000 })]`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export type Middleware =
  | TraceMiddleware
  | ApproveMiddleware
  | BudgetMiddleware
  | ContextInjectionMiddleware
  | PeerMiddleware
  | SecretsProxyMiddleware

/**
 * Serializable middleware chain accepted by `compose()`.
 *
 * @example `const chain = middleware([trace(), approve({ scope: 'tool_calls' })])`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface MiddlewareChain {
  /** Stable discriminator for serialized middleware chains. */
  readonly kind: 'middleware'
  /** Ordered middleware list applied to the ACP channel. */
  readonly chain: readonly Middleware[]
}

/**
 * Options accepted by `Harness.start()` and topology `start()` helpers.
 *
 * @example `await harness.start({ serverUrl: 'http://127.0.0.1:4440', name: 'demo' })`
 *
 * @remarks Anthropic primitive: Harness.
 */
export interface StartOptions {
  /** Base URL for the Fireline host or control plane. */
  readonly serverUrl: string
  /** Optional bearer token forwarded to provisioning requests. */
  readonly token?: string
  /** Optional runtime name override for this launch. */
  readonly name?: string
  /** Optional durable state stream name shared across launches. */
  readonly stateStream?: string
  /** Reserved for future startup timeout wiring. */
  readonly startupTimeoutMs?: number
}

/**
 * Runnable harness specification produced by `compose()`.
 *
 * @example `const spec: HarnessSpec<'default'> = compose(sandbox(), middleware([]), agent(['node', 'agent.mjs'])).spec`
 *
 * @remarks Anthropic primitive: Harness.
 */
export interface HarnessSpec<Name extends string = string> {
  /** Stable discriminator for serialized harness configs. */
  readonly kind: 'harness'
  /** Logical harness name used in stream names and future topologies. */
  readonly name: Name
  /** Sandbox definition used to provision the execution environment. */
  readonly sandbox: SandboxDefinition
  /** Middleware chain wired into the ACP path. */
  readonly middleware: MiddlewareChain
  /** Agent process definition launched inside the sandbox. */
  readonly agent: AgentConfig
  /** Optional explicit durable state stream name. */
  readonly stateStream?: string
}

/**
 * Public alias for the config accepted by `Sandbox.provision()`.
 *
 * @example `const config: SandboxConfig = compose(sandbox(), middleware([trace()]), agent(['node', 'agent.mjs'])).spec`
 *
 * @remarks Anthropic primitive: Harness.
 */
export type SandboxConfig<Name extends string = string> = HarnessSpec<Name>

/**
 * Backwards-compatible alias for serialized harness specs.
 *
 * @remarks Anthropic primitive: Harness.
 */
export type HarnessConfig<Name extends string = string> = HarnessSpec<Name>

/**
 * Minimal handle returned after provisioning succeeds.
 *
 * @example `const handle: SandboxHandle = await client.provision(config)`
 *
 * @remarks Anthropic primitive: Sandbox.
 */
export interface SandboxHandle {
  /** Provider-assigned sandbox identifier. */
  readonly id: string
  /** Provider name that created the sandbox. */
  readonly provider: string
  /** ACP endpoint used by the third-party ACP SDK. */
  readonly acp: Endpoint
  /** Durable state endpoint used by `@fireline/state`. */
  readonly state: Endpoint
}

/**
 * Harness-scoped handle returned by `Harness.start()`.
 *
 * @example `const handle = await harness.start({ serverUrl })`
 *
 * @remarks Anthropic primitive: Harness.
 */
export interface HarnessHandle<Name extends string = string> extends SandboxHandle {
  /** Logical harness name used when the handle was launched. */
  readonly name: Name
}

/**
 * Runnable harness value created by `compose()`.
 *
 * @example `const reviewer = compose(sandbox(), middleware([trace()]), agent(['agent'])).as('reviewer')`
 *
 * @remarks Anthropic primitive: Harness.
 */
export interface Harness<Name extends string = string> extends HarnessSpec<Name> {
  /** Returns a renamed harness while preserving sandbox, middleware, and agent config. */
  as<NextName extends string>(name: NextName): Harness<NextName>
  /** Provisions the harness and returns ACP/state endpoints. */
  start(options: StartOptions): Promise<HarnessHandle<Name>>
}

/**
 * Rich sandbox record returned by admin reads.
 *
 * @example `const descriptor: SandboxDescriptor | null = await admin.get('sandbox-1')`
 *
 * @remarks Anthropic primitive: Sandbox.
 */
export interface SandboxDescriptor extends SandboxHandle {
  /** Latest reported lifecycle state for the sandbox. */
  readonly status: SandboxStatus
  /** Labels currently associated with the sandbox. */
  readonly labels: Readonly<Record<string, string>>
  /** Creation timestamp in epoch milliseconds. */
  readonly createdAtMs: number
  /** Last update timestamp in epoch milliseconds. */
  readonly updatedAtMs: number
}
