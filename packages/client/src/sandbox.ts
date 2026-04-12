import { requestControlPlane } from './control-plane.js'
import type {
  AgentConfig,
  HarnessConfig,
  Middleware,
  SandboxConfig,
  SandboxDefinition,
  SandboxHandle,
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
   * @example `const handle = await client.provision(compose(sandbox(), [trace()], agent(['node', 'agent.mjs'])))`
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
 * @example `const cfg = sandbox({ resources: [], provider: 'local' })`
 *
 * @remarks Anthropic primitive: Sandbox.
 */
export function sandbox(config: Omit<SandboxDefinition, 'kind'> = {}): SandboxDefinition {
  return {
    kind: 'sandbox',
    ...cloneDefined(config),
  }
}

/**
 * Composes sandbox, middleware, and agent specs into a runnable harness config.
 *
 * @example `const config = compose(sandbox(), [trace()], agent(['node', 'agent.mjs']))`
 *
 * @remarks Anthropic primitive: Harness.
 */
export function compose(
  sandboxConfig: SandboxDefinition,
  middleware: readonly Middleware[],
  agentConfig: AgentConfig,
): HarnessConfig<'default'> {
  return {
    kind: 'harness',
    name: 'default',
    sandbox: sandboxConfig,
    middleware: [...middleware],
    agent: agentConfig,
  }
}

interface ProvisionRequest {
  readonly name: string
  readonly agentCommand: readonly string[]
  readonly topology: TopologySpec
  readonly resources: NonNullable<SandboxDefinition['resources']>
  readonly stateStream?: string
}

function buildProvisionRequest(config: SandboxConfig): ProvisionRequest {
  const name = config.name === 'default' ? `fireline-ts-${crypto.randomUUID()}` : config.name
  return {
    name,
    agentCommand: [...config.agent.command],
    topology: buildTopology(config.middleware, name),
    resources: [...(config.sandbox.resources ?? [])],
    stateStream: config.stateStream,
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
      return [{ name: 'peer_mcp' }]
  }
}

function cloneDefined<T extends object>(value: T): T {
  return Object.fromEntries(
    Object.entries(value).filter(([, entry]) => entry !== undefined),
  ) as T
}

export type { SandboxConfig, SandboxHandle }
