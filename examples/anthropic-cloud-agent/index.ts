import {
  Sandbox,
  agent,
  compose as composeHarness,
  sandbox,
  type AgentConfig,
  type Middleware as FirelineMiddleware,
  type SandboxDefinition,
  type SandboxHandle,
} from '@fireline/client'
import { approve, budget, trace } from '@fireline/client/middleware'
import { createFirelineDB, type FirelineDB } from '@fireline/state'

export type StartOptions = {
  readonly serverUrl: string
  readonly name: string
  readonly token?: string
}

export type StartedAnthropicCloudAgent = {
  readonly handle: SandboxHandle
  readonly db: FirelineDB
}

// Example-local sugar so the demo reads like the higher-level product pitch:
// compose(...).start(...) and middleware([...]).
export function middleware<const T extends readonly FirelineMiddleware[]>(entries: T): T {
  return entries
}

export function compose(
  sandboxConfig: SandboxDefinition,
  chain: readonly FirelineMiddleware[],
  agentConfig: AgentConfig,
) {
  const config = composeHarness(sandboxConfig, chain, agentConfig)

  return {
    config,
    async start(options: StartOptions): Promise<SandboxHandle> {
      return new Sandbox({
        serverUrl: options.serverUrl,
        token: options.token,
      }).provision({
        ...config,
        name: options.name,
      })
    },
  }
}

export async function startAnthropicCloudAgent(
  options: Partial<StartOptions> = {},
): Promise<StartedAnthropicCloudAgent> {
  const handle = await compose(
    sandbox({ provider: 'anthropic' }),
    middleware([
      trace(),
      approve({ scope: 'tool_calls' }),
      budget({ tokens: 500_000 }),
    ]),
    agent(['claude-sonnet-4-6']),
  ).start({
    serverUrl: options.serverUrl ?? 'http://localhost:4440',
    name: options.name ?? 'anthropic-cloud-agent',
    token: options.token,
  })

  // ACP / session plane — same as any other Fireline provider.
  // Anthropic's managed-agent activity is bridged back into the ACP model.
  // Use @agentclientprotocol/sdk against handle.acp.url.
  console.log('ACP endpoint:', handle.acp.url)

  // State observation — @fireline/state works regardless of provider.
  const db = createFirelineDB({ stateStreamUrl: handle.state.url })
  await db.preload()

  return { handle, db }
}

export async function stopObservedAgent(agentRun: StartedAnthropicCloudAgent): Promise<void> {
  agentRun.db.close()
}
