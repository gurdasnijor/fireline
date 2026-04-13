import { FirelineAgent } from './agent.js'
import {
  connectSpawnedStdio,
  connectStream,
  connectWebSocket,
  type ConnectedAcp,
} from './connect.js'
import { requestControlPlane } from './control-plane.js'
import type {
  AgentConfig,
  CapabilityRef,
  Conductor,
  ConductorSpec,
  ConductorTransport,
  CredentialRef,
  DurableSubscriberEventSelector,
  HostedTransport,
  MiddlewareChain,
  Middleware,
  SandboxConfig,
  SandboxDefinition,
  SandboxHandle,
  StartOptions,
  TopologyComponentSpec,
  TopologySpec,
  ToolAttachment,
  TransportRef,
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
 * Composes sandbox, middleware, and agent specs into a runnable conductor value.
 *
 * @example `const conductor = compose(sandbox(), middleware([trace()]), agent(['node', 'agent.mjs']))`
 *
 * @remarks Anthropic primitive: Conductor.
 */
export function compose(
  sandboxConfig: SandboxDefinition,
  middlewareConfig: MiddlewareChain,
  agentConfig: AgentConfig,
): Conductor<'default'> {
  return createConductor({
    kind: 'conductor',
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
    topology: buildTopology(
      config.middleware.chain,
      name,
      config.sandbox.fsBackend,
    ),
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

function buildTopology(
  middleware: readonly Middleware[],
  name: string,
  fsBackend?: SandboxDefinition['fsBackend'],
): TopologySpec {
  const components = middleware.flatMap((entry) => middlewareToComponents(entry, name))
  if (fsBackend) {
    components.push({
      name: 'fs_backend',
      config: fsBackendToConfig(fsBackend),
    })
  }
  return { components }
}

function fsBackendToConfig(
  fsBackend: NonNullable<SandboxDefinition['fsBackend']>,
): Record<string, string> {
  switch (fsBackend) {
    case 'local':
      return { backend: 'local' }
    case 'streamFs':
      return { backend: 'runtime_stream' }
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
      if (middleware.scope === 'tool_calls') {
        return [
          {
            name: 'approval_gate',
            config: {
              ...(middleware.timeoutMs ? { timeoutMs: middleware.timeoutMs } : {}),
              policies: [
                {
                  match: { kind: 'toolPrefix', prefix: '' },
                  action: 'requireApproval',
                  reason: 'approval required for tool call',
                },
              ],
            },
          },
        ]
      }

      return [
        {
          name: 'approval_gate',
          config: {
            ...(middleware.timeoutMs ? { timeoutMs: middleware.timeoutMs } : {}),
            policies: [
              {
                match: { kind: 'promptContains', needle: '' },
                action: 'requireApproval',
                reason: 'approval required for every prompt',
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
    case 'attachTools':
      return [
        {
          name: 'attach_tool',
          config: {
            capabilities: middleware.tools.map(normalizeCapabilityRef),
          },
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
    case 'webhook':
      return [
        {
          name: 'webhook_subscriber',
          config: buildWebhookSubscriberConfig(middleware),
        },
      ]
    case 'telegram':
      return [
        {
          name: 'telegram',
          config: buildTelegramSubscriberConfig(middleware),
        },
      ]
    case 'autoApprove':
      return [
        {
          name: 'auto_approve',
        },
      ]
    case 'peerRouting':
      return [
        {
          name: 'peer_routing',
          ...(middleware.name ? { config: { name: middleware.name } } : {}),
        },
      ]
    case 'wakeDeployment':
      return [
        {
          name: 'always_on_deployment',
          ...(middleware.name ? { config: { name: middleware.name } } : {}),
        },
      ]
  }
}

function cloneSubscriberEventSelector(
  selector: DurableSubscriberEventSelector,
): DurableSubscriberEventSelector {
  return typeof selector === 'string' ? selector : { ...selector }
}

function buildWebhookSubscriberConfig(middleware: Extract<Middleware, { kind: 'webhook' }>) {
  const target = middleware.target ?? middleware.name ?? deriveWebhookTarget(middleware.url)
  const maxAttempts = middleware.retry?.maxAttempts ?? 1
  return {
    target,
    events: middleware.events.map(normalizeWebhookEventSelector),
    targetConfig: {
      url: middleware.url,
      headers: middleware.headers
        ? Object.fromEntries(
            Object.entries(middleware.headers).map(([headerName, ref]) => [
              headerName,
              ref.ref,
            ]),
          )
        : {},
      timeoutMs: 5_000,
      maxAttempts,
      cursorStream: `subscribers:webhook:${slugWebhookTarget(target)}`,
      deadLetterStream: `subscribers:webhook:${slugWebhookTarget(target)}:dead-letter`,
    },
    ...(middleware.retry ? { retryPolicy: buildRetryPolicy(middleware.retry) } : {}),
  }
}

// Minimum-correct for the current Rust TelegramSubscriber surface. Full
// DurableSubscriber parity for keyBy/cursorStream/deadLetterStream/retry is
// pending mono-axr.11 follow-ups that close DSV-03/04/05 on the Rust side.
function buildTelegramSubscriberConfig(
  middleware: Extract<Middleware, { kind: 'telegram' }>,
) {
  return cloneDefined({
    botToken:
      typeof middleware.token === 'string'
        ? middleware.token
        : middleware.token?.ref,
    apiBaseUrl: middleware.apiBaseUrl ?? 'https://api.telegram.org',
    chatId: middleware.chatId,
    allowedUserIds: middleware.allowedUserIds
      ? [...middleware.allowedUserIds]
      : [],
    approvalTimeoutMs: middleware.approvalTimeoutMs,
    pollIntervalMs: middleware.pollIntervalMs ?? 1_000,
    pollTimeoutMs: middleware.pollTimeoutMs ?? 30_000,
    parseMode: normalizeTelegramParseMode(middleware.parseMode ?? 'html'),
    scope: normalizeTelegramScope(middleware.scope ?? 'tool_calls'),
  })
}

function normalizeWebhookEventSelector(
  selector: DurableSubscriberEventSelector,
) {
  if (typeof selector === 'string') {
    return { kind: selector }
  }

  return {
    exact: {
      entityType: selector.type,
      ...(selector.kind ? { kind: selector.kind } : {}),
    },
  }
}

function buildRetryPolicy(retry: NonNullable<Extract<Middleware, { kind: 'webhook' }>['retry']>) {
  return cloneDefined({
    maxAttempts: retry.maxAttempts,
    initialBackoffMs: retry.initialBackoffMs,
    maxBackoffMs: retry.maxBackoffMs ?? retry.initialBackoffMs,
  })
}

function deriveWebhookTarget(url: string | undefined): string {
  if (!url) {
    return 'webhook'
  }

  try {
    return new URL(url).host
  } catch {
    return 'webhook'
  }
}

function slugWebhookTarget(target: string): string {
  return target.replace(/[^a-zA-Z0-9._-]+/g, '-')
}

function normalizeTelegramParseMode(parseMode: 'html' | 'markdown_v2') {
  switch (parseMode) {
    case 'html':
      return 'html'
    case 'markdown_v2':
      return 'markdown_v2'
  }
}

function normalizeTelegramScope(scope: 'tool_calls') {
  switch (scope) {
    case 'tool_calls':
      return 'tool_calls'
  }
}

function normalizeCapabilityRef(
  capability: ToolAttachment | CapabilityRef,
): CapabilityRef {
  if ('descriptor' in capability) {
    return {
      descriptor: {
        name: capability.descriptor.name,
        description: capability.descriptor.description,
        inputSchema: capability.descriptor.inputSchema,
      },
      transportRef: cloneTransportRef(capability.transportRef),
      ...(capability.credentialRef
        ? { credentialRef: cloneCredentialRef(capability.credentialRef) }
        : {}),
    }
  }

  return {
    descriptor: {
      name: capability.name,
      description: capability.description ?? '',
      inputSchema: capability.inputSchema ?? { type: 'object' },
    },
    transportRef: parseTransportRef(capability.transport),
    ...(capability.credential
      ? { credentialRef: parseCredentialRef(capability.credential) }
      : {}),
  }
}

function parseTransportRef(ref: string | TransportRef): TransportRef {
  if (typeof ref !== 'string') {
    return cloneTransportRef(ref)
  }

  if (ref.startsWith('mcp:')) {
    const url = ref.slice(4)
    if (!url) {
      throw new Error("invalid transport ref 'mcp:': missing MCP URL")
    }
    return { kind: 'mcpUrl', url }
  }

  if (ref.startsWith('peer:')) {
    const hostKey = ref.slice(5)
    if (!hostKey) {
      throw new Error("invalid transport ref 'peer:': missing host key")
    }
    return { kind: 'peerRuntime', hostKey }
  }

  if (ref.startsWith('smithery:')) {
    const [catalog, tool, extra] = ref.slice(9).split(':')
    if (!catalog || !tool || extra) {
      throw new Error(
        `invalid transport ref '${ref}': expected smithery:<catalog>:<tool>`,
      )
    }
    return { kind: 'smithery', catalog, tool }
  }

  if (ref.startsWith('inprocess:')) {
    const componentName = ref.slice(10)
    if (!componentName) {
      throw new Error(
        "invalid transport ref 'inprocess:': missing component name",
      )
    }
    return { kind: 'inProcess', componentName }
  }

  if (ref.startsWith('component:')) {
    const componentName = ref.slice(10)
    if (!componentName) {
      throw new Error(
        "invalid transport ref 'component:': missing component name",
      )
    }
    return { kind: 'inProcess', componentName }
  }

  return { kind: 'mcpUrl', url: ref }
}

function parseCredentialRef(ref: string | CredentialRef): CredentialRef {
  if (typeof ref !== 'string') {
    return cloneCredentialRef(ref)
  }

  if (ref.startsWith('env:')) {
    const variable = ref.slice(4)
    if (!variable) {
      throw new Error(
        `invalid env credential ref '${ref}': missing variable name`,
      )
    }
    return { kind: 'env', var: variable }
  }

  if (ref.startsWith('secret:')) {
    const key = ref.slice(7)
    if (!key) {
      throw new Error(`invalid secret credential ref '${ref}': missing key`)
    }
    return { kind: 'secret', key }
  }

  if (ref.startsWith('oauth:')) {
    const parts = ref.slice(6).split(':')
    const provider = parts[0] ?? ''
    const account = parts[1]
    if (!provider || parts.length > 2) {
      throw new Error(
        `invalid oauth credential ref '${ref}': expected oauth:<provider>[:account]`,
      )
    }
    return cloneDefined({
      kind: 'oauthToken' as const,
      provider,
      account,
    })
  }

  throw new Error(
    `unsupported credential ref '${ref}': expected env:, secret:, or oauth:`,
  )
}

function cloneTransportRef(ref: TransportRef): TransportRef {
  switch (ref.kind) {
    case 'peerRuntime':
      return { kind: 'peerRuntime', hostKey: ref.hostKey }
    case 'smithery':
      return { kind: 'smithery', catalog: ref.catalog, tool: ref.tool }
    case 'mcpUrl':
      return { kind: 'mcpUrl', url: ref.url }
    case 'inProcess':
      return { kind: 'inProcess', componentName: ref.componentName }
  }
}

function cloneCredentialRef(ref: CredentialRef): CredentialRef {
  switch (ref.kind) {
    case 'env':
      return { kind: 'env', var: ref.var }
    case 'secret':
      return { kind: 'secret', key: ref.key }
    case 'oauthToken':
      return cloneDefined({
        kind: 'oauthToken' as const,
        provider: ref.provider,
        account: ref.account,
      })
  }
}

function cloneDefined<T extends object>(value: T): T {
  return Object.fromEntries(
    Object.entries(value).filter(([, entry]) => entry !== undefined),
  ) as T
}

export type { SandboxConfig, SandboxHandle }

function createConductor<
  Name extends string,
  Role extends 'client' | 'agent' = 'client',
>(spec: ConductorSpec<Name>, role: Role = 'client' as Role): Conductor<Name, Role> {
  return {
    ...spec,
    role,
    as<NextName extends string>(name: NextName): Conductor<NextName, Role> {
      return createConductor({
        ...spec,
        name,
      }, role)
    },
    asRole<NextRole extends 'client' | 'agent'>(
      nextRole: NextRole,
    ): Conductor<Name, NextRole> {
      return createConductor({
        ...spec,
      }, nextRole)
    },
    async connect_to(
      transport: ConductorTransport<Role>,
    ): Promise<Role extends 'client' ? ConnectedAcp : never> {
      if (role !== 'client') {
        throw new Error(
          'agent-facing TypeScript conductors are reserved for a future proxy-chain rollout; connect_to currently supports client-facing conductors only',
        )
      }

      switch ((transport as ConductorTransport<'client'>).kind) {
        case 'hosted':
          return connectHostedConductor(
            spec,
            transport as Extract<ConductorTransport<'client'>, { readonly kind: 'hosted' }>,
          ) as Promise<Role extends 'client' ? ConnectedAcp : never>
        case 'websocket':
          return connectWebSocket(
            transport as Extract<ConductorTransport<'client'>, { readonly kind: 'websocket' }>,
            (transport as Extract<ConductorTransport<'client'>, { readonly kind: 'websocket' }>).clientName,
          ) as Promise<Role extends 'client' ? ConnectedAcp : never>
        case 'stream':
          return connectStream(
            (transport as Extract<ConductorTransport<'client'>, { readonly kind: 'stream' }>).stream,
            (transport as Extract<ConductorTransport<'client'>, { readonly kind: 'stream' }>).clientName,
          ) as Promise<Role extends 'client' ? ConnectedAcp : never>
        case 'stdio':
          return connectStdioConductor(
            spec,
            transport as Extract<ConductorTransport<'client'>, { readonly kind: 'stdio' }>,
          ) as Promise<Role extends 'client' ? ConnectedAcp : never>
      }
    },
    /**
     * @deprecated Prefer `connect_to({ kind: 'hosted', ... })`.
     */
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
        name: name as Name,
        handle,
      })
    },
  }
}

async function connectHostedConductor<Name extends string>(
  spec: ConductorSpec<Name>,
  transport: HostedTransport,
): Promise<ConnectedAcp> {
  const name = transport.name ?? spec.name
  const handle = await new Sandbox({
    serverUrl: transport.url,
    token: transport.token,
  }).provision({
    ...spec,
    name,
    stateStream: transport.stateStream ?? spec.stateStream,
  })
  const agentHandle = new FirelineAgent({
    serverUrl: transport.url,
    token: transport.token,
    name,
    handle,
  })
  const connection = await agentHandle.connect(transport.clientName)
  const close = connection.close.bind(connection)

  return Object.assign(connection, {
    async close() {
      const errors: unknown[] = []
      try {
        await close()
      } catch (error) {
        errors.push(error)
      }
      try {
        await agentHandle.stop()
      } catch (error) {
        errors.push(error)
      }
      if (errors.length > 0) {
        throw errors[0]
      }
    },
  })
}

async function connectStdioConductor<Name extends string>(
  spec: ConductorSpec<Name>,
  transport: Extract<ConductorTransport<'client'>, { readonly kind: 'stdio' }>,
): Promise<ConnectedAcp> {
  const durableStreamsUrl =
    transport.durableStreamsUrl ?? process.env.FIRELINE_DURABLE_STREAMS_URL
  if (!durableStreamsUrl) {
    throw new Error(
      'stdio conductor transport requires durableStreamsUrl (or FIRELINE_DURABLE_STREAMS_URL)',
    )
  }

  const firelineBin = transport.firelineBin ?? process.env.FIRELINE_BIN ?? 'fireline'
  const args = [
    '--acp-stdio',
    '--host',
    transport.host ?? '127.0.0.1',
    '--port',
    String(transport.port ?? 0),
    '--name',
    transport.name ?? spec.name,
    '--durable-streams-url',
    durableStreamsUrl,
    '--topology-json',
    JSON.stringify(
      buildTopology(
        spec.middleware.chain,
        transport.name ?? spec.name,
        spec.sandbox.fsBackend,
      ),
    ),
  ]

  const mountedResources = await lowerMountedResources(spec.sandbox.resources)
  if (mountedResources.length > 0) {
    args.push('--mounted-resources-json', JSON.stringify(mountedResources))
  }
  const stateStream = transport.stateStream ?? spec.stateStream
  if (stateStream) {
    args.push('--state-stream', stateStream)
  }
  if (transport.peerDirectoryPath) {
    args.push('--peer-directory-path', transport.peerDirectoryPath)
  }
  args.push('--', ...spec.agent.command)

  return connectSpawnedStdio(
    {
      command: firelineBin,
      args,
      cwd: transport.cwd,
      env: transport.env,
    },
    transport.clientName,
  )
}

interface MountedResourceArg {
  readonly hostPath: string
  readonly mountPath: string
  readonly readOnly: boolean
}

function lowerMountedResources(
  resources: SandboxDefinition['resources'],
): Promise<MountedResourceArg[]> {
  return Promise.all((resources ?? []).map(async (resource) => {
    if (resource.source_ref.kind !== 'localPath') {
      throw new Error(
        `stdio conductor transport currently supports only localPath resources; received ${resource.source_ref.kind}`,
      )
    }

    return {
      hostPath: await resolveLocalResourcePath(resource.source_ref.path),
      mountPath: resource.mount_path,
      readOnly: resource.read_only ?? false,
    }
  }))
}

async function resolveLocalResourcePath(path: string): Promise<string> {
  const { realpath } = await import('node:fs/promises')
  const { resolve } = await import('node:path')
  try {
    return await realpath(path)
  } catch {
    return resolve(path)
  }
}
