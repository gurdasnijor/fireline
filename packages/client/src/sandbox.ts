import { requestControlPlane } from './control-plane.js'
import type { TopologyComponentSpec, TopologySpec } from './topology.js'
import type {
  AgentConfig,
  HarnessConfig,
  Middleware,
  SandboxConfig,
  SandboxDefinition,
  SandboxHandle,
} from './types.js'

export interface SandboxClientOptions {
  readonly serverUrl: string
  readonly token?: string
}

export class Sandbox {
  readonly serverUrl: string
  readonly token?: string

  constructor(options: SandboxClientOptions) {
    this.serverUrl = options.serverUrl
    this.token = options.token
  }

  async provision(config: SandboxConfig): Promise<SandboxHandle> {
    const request = buildProvisionRequest(config)

    try {
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
    } catch (error) {
      if (!isMissingEndpoint(error)) {
        throw error
      }
    }

    const handle = await requestControlPlane<SandboxHandle>(
      this,
      '/v1/runtimes',
      {
        method: 'POST',
        body: JSON.stringify(request),
      },
    )
    if (!handle) {
      throw new Error('control plane returned an empty runtime handle')
    }
    return handle
  }
}

export function agent(command: readonly string[]): AgentConfig {
  return {
    kind: 'agent',
    command: [...command],
  }
}

export function sandbox(config: Omit<SandboxDefinition, 'kind'> = {}): SandboxDefinition {
  return {
    kind: 'sandbox',
    ...cloneDefined(config),
  }
}

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

function isMissingEndpoint(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'status' in error &&
    (error.status === 404 || error.status === 405)
  )
}

function cloneDefined<T extends object>(value: T): T {
  return Object.fromEntries(
    Object.entries(value).filter(([, entry]) => entry !== undefined),
  ) as T
}

export type { SandboxConfig, SandboxHandle }
