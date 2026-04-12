import { FirelineAgent } from './agent.js'
import { requestControlPlane } from './control-plane.js'
import type {
  AgentConfig,
  Harness,
  HarnessSpec,
  MiddlewareChain,
  Middleware,
  SandboxConfig,
  SandboxDefinition,
  SandboxHandle,
  StartOptions,
  TopologyComponentSpec,
  TopologySpec,
} from './types.js'

/**
 * Connection settings for the Fireline host that provisions sandboxes.
 *
 * @example `const client = new Sandbox({ serverUrl: 'http://127.0.0.1:4440' })`
 *
 * @remarks Anthropic primitive: Sandbox.
 */
export interface SandboxClientOptions {
  /** Base URL for the Fireline host or control plane. */
  readonly serverUrl: string
  /** Optional bearer token forwarded to the host on every request. */
  readonly token?: string
}

/**
 * Control-plane client for provisioning sandboxes from harness configs.
 *
 * @example `const handle = await new Sandbox({ serverUrl }).provision(config)`
 *
 * @remarks Anthropic primitive: Sandbox.
 */
export class Sandbox {
  /** Base URL for the Fireline host or control plane. */
  readonly serverUrl: string
  /** Optional bearer token forwarded to the host on every request. */
  readonly token?: string

  constructor(options: SandboxClientOptions) {
    this.serverUrl = options.serverUrl
    this.token = options.token
  }

  /**
   * Provisions a sandbox for the supplied harness config and returns ACP/state endpoints.
   *
   * @example `const handle = await client.provision(compose(sandbox(), middleware([trace()]), agent(['node', 'agent.mjs'])).spec)`
   *
   * @remarks Anthropic primitive: Sandbox.
   */
  async provision(config: SandboxConfig): Promise<SandboxHandle> {
    const request = buildProvisionRequest(config)
    const handle = await requestControlPlane<SandboxHandle>(
      this,
      '/v1/sandboxes',
      {
        method: 'POST',
        body: JSON.stringify(request),
      },
    )
    if (!handle) {
      throw new Error('control plane returned an empty sandbox handle')
    }
    return handle
  }
}

/**
 * Creates a serializable agent process definition for `compose()`.
 *
 * @example `const cfg = agent(['npx', '-y', '@anthropic-ai/claude-code-acp'])`
 *
 * @remarks Anthropic primitive: Harness.
 */
export function agent(command: readonly string[]): AgentConfig {
  return {
    kind: 'agent',
    command: [...command],
  }
}

/**
 * Creates a serializable sandbox definition for `compose()`.
 *
 * @example `const cfg = sandbox({ resources: [], provider: 'docker', image: 'node:22-slim' })`
 *
 * @remarks Anthropic primitive: Sandbox.
 */
export function sandbox(
  config: SandboxDefinitionOptions = {},
): SandboxDefinition {
  return {
    kind: 'sandbox',
    ...cloneDefined(config),
  }
}

/**
 * Wraps a middleware array in a serializable middleware-chain value.
 *
 * @example `const chain = middleware([trace(), approve({ scope: 'tool_calls' })])`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export function middleware(chain: readonly Middleware[]): MiddlewareChain {
  return {
    kind: 'middleware',
    chain: [...chain],
  }
}

/**
 * Composes sandbox, middleware, and agent specs into a runnable harness value.
 *
 * @example `const harness = compose(sandbox(), middleware([trace()]), agent(['node', 'agent.mjs']))`
 *
 * @remarks Anthropic primitive: Harness.
 */
export function compose(
  sandboxConfig: SandboxDefinition,
  middlewareConfig: MiddlewareChain,
  agentConfig: AgentConfig,
): Harness<'default'> {
  return createHarness({
    kind: 'harness',
    name: 'default',
    sandbox: sandboxConfig,
    middleware: middlewareConfig,
    agent: agentConfig,
  })
}

interface ProvisionRequest {
  readonly name: string
  readonly agentCommand: readonly string[]
  readonly topology: TopologySpec
  readonly resources: NonNullable<SandboxDefinition['resources']>
  readonly envVars?: Readonly<Record<string, string>>
  readonly labels?: Readonly<Record<string, string>>
  readonly provider?: string
  readonly image?: string
  readonly model?: string
  readonly stateStream?: string
}

type SandboxDefinitionOptions = Omit<SandboxDefinition, 'kind'>

function buildProvisionRequest(config: SandboxConfig): ProvisionRequest {
  const name = config.name === 'default' ? `fireline-ts-${crypto.randomUUID()}` : config.name
  const provider = resolveProviderConfig(config.sandbox)
  return {
    name,
    agentCommand: [...config.agent.command],
    topology: buildTopology(config.middleware.chain, name),
    resources: [...(config.sandbox.resources ?? [])],
    envVars: config.sandbox.envVars,
    labels: config.sandbox.labels,
    ...provider,
    stateStream: config.stateStream,
  }
}

function resolveProviderConfig(
  sandbox: SandboxDefinition,
): Pick<ProvisionRequest, 'provider' | 'image' | 'model'> {
  switch (sandbox.provider) {
    case 'docker':
      return cloneDefined({
        provider: 'docker',
        image: sandbox.image,
      })
    case 'microsandbox':
      return { provider: 'microsandbox' }
    case 'anthropic':
      return cloneDefined({
        provider: 'anthropic',
        model: sandbox.model,
      })
    case 'local':
      return { provider: 'local' }
    default:
      return {}
  }
}

function buildTopology(middleware: readonly Middleware[], name: string): TopologySpec {
  return {
    components: middleware.flatMap((entry) => middlewareToComponents(entry, name)),
  }
}

function middlewareToComponents(middleware: Middleware, name: string): TopologyComponentSpec[] {
  switch (middleware.kind) {
    case 'trace':
      return [
        {
          name: 'audit',
          config: {
            streamName: middleware.streamName ?? `audit:${name}`,
            ...(middleware.includeMethods ? { includeMethods: [...middleware.includeMethods] } : {}),
          },
        },
      ]
    case 'approve':
      return [
        {
          name: 'approval_gate',
          config: {
            ...(middleware.timeoutMs ? { timeoutMs: middleware.timeoutMs } : {}),
            policies: [
              {
                match: { kind: 'promptContains', needle: '' },
                action: 'requireApproval',
                reason:
                  middleware.scope === 'tool_calls'
                    ? 'approval fallback: prompt-level gate until tool-call interception lands'
                    : 'approval required for every prompt',
              },
            ],
          },
        },
      ]
    case 'budget':
      return [
        {
          name: 'budget',
          config: {
            ...(middleware.tokens !== undefined ? { maxTokens: middleware.tokens } : {}),
          },
        },
      ]
    case 'contextInjection':
      return [
        {
          name: 'context_injection',
          config: cloneDefined({
            prependText: middleware.prependText,
            placement: middleware.placement,
            sources: middleware.sources ? [...middleware.sources] : undefined,
          }),
        },
      ]
    case 'peer':
      return [
        {
          name: 'peer_mcp',
          ...(middleware.peers?.length ? { config: { peers: [...middleware.peers] } } : {}),
        },
      ]
    case 'secretsProxy':
      return [
        {
          name: 'secrets_injection',
          config: {
            bindings: Object.entries(middleware.bindings).map(([name, binding]) => ({
              name,
              ref: binding.ref,
              ...(binding.allow ? { allow: Array.isArray(binding.allow) ? [...binding.allow] : [binding.allow] } : {}),
            })),
          },
        },
      ]
  }
}

function cloneDefined<T extends object>(value: T): T {
  return Object.fromEntries(
    Object.entries(value).filter(([, entry]) => entry !== undefined),
  ) as T
}

export type { SandboxConfig, SandboxHandle }

function createHarness<Name extends string>(spec: HarnessSpec<Name>): Harness<Name> {
  return {
    ...spec,
    as<NextName extends string>(name: NextName): Harness<NextName> {
      return createHarness({
        ...spec,
        name,
      })
    },
    async start(options: StartOptions): Promise<FirelineAgent<Name>> {
      const name = options.name ?? spec.name
      const handle = await new Sandbox(options).provision({
        ...spec,
        name,
        stateStream: options.stateStream ?? spec.stateStream,
      })
      return new FirelineAgent({
        serverUrl: options.serverUrl,
        token: options.token,
        name: spec.name,
        handle,
      })
    },
  }
}
