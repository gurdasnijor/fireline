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

  async provision(_config: SandboxConfig): Promise<SandboxHandle> {
    throw new Error('Sandbox.provision() is wired in phase 2')
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

function cloneDefined<T extends object>(value: T): T {
  return Object.fromEntries(
    Object.entries(value).filter(([, entry]) => entry !== undefined),
  ) as T
}

export type { SandboxConfig, SandboxHandle }
