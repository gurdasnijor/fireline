import type { FirelineAgent } from './agent.js'
import type { ConnectedAcp } from './connect.js'
import type { ResourceRef } from './resources.js'
import type { Stream } from '@agentclientprotocol/sdk'

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
 * Provider-specific sandbox selection and launch config.
 *
 * This mirrors the provider names Fireline's Rust host understands today while
 * keeping provider-only fields scoped to the matching variant.
 *
 * @remarks Anthropic primitive: Sandbox.
 */
export type SandboxProviderConfig =
  | {
      /** Default local child-process provider. Omit entirely to use the host default. */
      readonly provider?: 'local'
    }
  | {
      /** Docker-backed sandbox provider. */
      readonly provider: 'docker'
      /** Optional OCI image hint forwarded to the host. */
      readonly image?: string
    }
  | {
      /** Microsandbox-backed provider. */
      readonly provider: 'microsandbox'
    }
  | {
      /** Anthropic managed-agents provider. */
      readonly provider: 'anthropic'
      /** Optional Anthropic model hint forwarded to the host. */
      readonly model?: string
    }

type SandboxDefinitionBase = {
  /** Resources mounted into the sandbox before the agent starts. */
  readonly resources?: readonly ResourceRef[]
  /** Environment variables that the provider may inject into the sandbox. */
  readonly envVars?: Readonly<Record<string, string>>
  /** Optional filesystem backend used by ACP file helpers inside the sandbox. */
  readonly fsBackend?: 'local' | 'streamFs'
  /** Labels used for lookup, routing, or fleet bookkeeping. */
  readonly labels?: Readonly<Record<string, string>>
}

/**
 * Serializable sandbox recipe that describes the execution environment.
 *
 * @example `const cfg: SandboxDefinition = { kind: 'sandbox', resources: [] }`
 *
 * @remarks Anthropic primitive: Sandbox.
 */
export type SandboxDefinition = SandboxDefinitionBase &
  SandboxProviderConfig & {
  /** Stable discriminator for serialized sandbox definitions. */
  readonly kind: 'sandbox'
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
  readonly scope: 'tool_calls' | 'prompts' | 'all'
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
 * Anthropic-shaped tool descriptor advertised to the agent.
 *
 * @example `const descriptor: ToolDescriptor = { name: 'github', description: 'GitHub MCP tools', inputSchema: { type: 'object' } }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface ToolDescriptor {
  /** Stable tool name exposed to the agent. */
  readonly name: string
  /** Human-readable description shown by ACP-capable clients. */
  readonly description: string
  /** JSON Schema describing the tool input payload. */
  readonly inputSchema: Record<string, unknown>
}

/**
 * Portable transport handle describing where the conductor should fetch a tool.
 *
 * Mirrors `fireline_tools::TransportRef` on the Rust side.
 *
 * @example `const transport: TransportRef = { kind: 'mcpUrl', url: 'https://example.com/mcp' }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export type TransportRef =
  | { readonly kind: 'peerRuntime'; readonly hostKey: string }
  | { readonly kind: 'smithery'; readonly catalog: string; readonly tool: string }
  | { readonly kind: 'mcpUrl'; readonly url: string }
  | { readonly kind: 'inProcess'; readonly componentName: string }

/**
 * Portable credential handle resolved by the host at call time.
 *
 * Mirrors `fireline_tools::CredentialRef` on the Rust side.
 *
 * @example `const credential: CredentialRef = { kind: 'secret', key: 'gh-pat' }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export type CredentialRef =
  | { readonly kind: 'env'; readonly var: string }
  | { readonly kind: 'secret'; readonly key: string }
  | { readonly kind: 'oauthToken'; readonly provider: string; readonly account?: string }

/**
 * Rust-aligned capability handle consumed by the `attach_tool` topology component.
 *
 * @example `const capability: CapabilityRef = { descriptor, transportRef: { kind: 'mcpUrl', url: 'https://example.com/mcp' } }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface CapabilityRef {
  /** Agent-visible tool descriptor. */
  readonly descriptor: ToolDescriptor
  /** Conductor-facing transport reference used to fetch the tool. */
  readonly transportRef: TransportRef
  /** Optional credential reference resolved by the host. */
  readonly credentialRef?: CredentialRef
}

/**
 * Ergonomic shorthand for declaring a capability attachment in TypeScript.
 *
 * `attachTools()` expands this into the Rust wire shape expected by the
 * `attach_tool` topology component.
 *
 * @example `const tool: ToolAttachment = { name: 'github', transport: 'mcp:https://example.com/mcp', credential: 'secret:gh-pat' }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface ToolAttachment {
  /** Agent-visible tool name. */
  readonly name: string
  /** Optional human-readable description. Defaults to an empty string. */
  readonly description?: string
  /** Optional JSON Schema. Defaults to `{ type: 'object' }`. */
  readonly inputSchema?: Record<string, unknown>
  /** Tool transport reference, either as a shorthand string or structured ref. */
  readonly transport: string | TransportRef
  /** Optional credential reference, either as a shorthand string or structured ref. */
  readonly credential?: string | CredentialRef
}

/**
 * Middleware spec that attaches launch-time capabilities to the harness topology.
 *
 * @example `const mw: AttachToolsMiddleware = { kind: 'attachTools', tools: [{ name: 'github', transport: 'mcp:https://example.com/mcp' }] }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface AttachToolsMiddleware {
  /** Stable discriminator for capability-attachment middleware. */
  readonly kind: 'attachTools'
  /** Capability attachments declared for this harness. */
  readonly tools: readonly (ToolAttachment | CapabilityRef)[]
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
 * Declarative completion-key strategy for durable subscriber middleware.
 *
 * These map directly onto the Rust `CompletionKey` enum and intentionally
 * exclude user-supplied string keys.
 *
 * @example `const keyBy: DurableSubscriberKeyStrategy = 'session_request'`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export type DurableSubscriberKeyStrategy =
  | 'session'
  | 'session_request'
  | 'session_tool_call'

/**
 * Selector used by active durable subscribers to match agent-plane envelopes.
 *
 * String selectors match `value.kind` first and then fall back to envelope
 * `type`, matching the webhook profile shape described in
 * `docs/proposals/durable-subscriber.md`.
 *
 * @example `const events: DurableSubscriberEventSelector[] = ['permission_request', { type: 'permission', kind: 'approval_resolved' }]`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export type DurableSubscriberEventSelector =
  | string
  | {
      readonly type: string
      readonly kind?: string
    }

/**
 * Host-resolved secret reference used by durable subscriber profiles.
 *
 * The TypeScript surface stays declarative: subscriber middleware points at a
 * host-owned secret reference instead of embedding plaintext credentials.
 *
 * @example `const token: DurableSubscriberSecretRef = { ref: 'secret:telegram-bot' }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface DurableSubscriberSecretRef {
  /** Host-owned secret or environment reference such as `secret:foo` or `env:BAR`. */
  readonly ref: string
}

/**
 * Bounded retry policy for active durable subscriber profiles.
 *
 * @example `const retry: DurableSubscriberRetryPolicy = { maxAttempts: 5, initialBackoffMs: 1_000 }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface DurableSubscriberRetryPolicy {
  /** Maximum delivery attempts before dead-lettering in the infrastructure plane. */
  readonly maxAttempts?: number
  /** Initial retry backoff in milliseconds. */
  readonly initialBackoffMs?: number
  /** Optional maximum retry backoff in milliseconds. */
  readonly maxBackoffMs?: number
}

/**
 * Middleware spec for the active `WebhookSubscriber` durable-subscriber profile.
 *
 * This is declarative config only. The Rust side lowers it onto an
 * `ActiveSubscriber`; no custom completion keys are accepted in userland.
 *
 * Today the live Rust substrate requires a concrete `url`; host-owned target
 * aliases can still be carried for naming, but target-only delivery remains a
 * follow-on host-wiring step.
 *
 * @example `const mw: WebhookMiddleware = { kind: 'webhook', url: 'https://hooks.slack.com/services/demo', events: ['permission_request'], keyBy: 'session_request' }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface WebhookMiddleware {
  /** Stable discriminator for webhook durable-subscriber middleware. */
  readonly kind: 'webhook'
  /** Optional human-readable profile name used in logs and diagnostics. */
  readonly name?: string
  /** Preferred host-owned webhook target alias. */
  readonly target?: string
  /** Direct delivery URL used by the current Rust `WebhookSubscriberConfig`. */
  readonly url?: string
  /** Event selectors consumed by the Rust `WebhookSubscriber` profile. */
  readonly events: readonly DurableSubscriberEventSelector[]
  /** Canonical completion-key strategy; raw string keys are intentionally unsupported. */
  readonly keyBy?: Exclude<DurableSubscriberKeyStrategy, 'session'>
  /** Optional host-resolved secret headers forwarded on delivery. */
  readonly headers?: Readonly<Record<string, DurableSubscriberSecretRef>>
  /** Optional bounded retry policy owned by the infrastructure plane. */
  readonly retry?: DurableSubscriberRetryPolicy
}

/**
 * Middleware spec for a Telegram-flavored active durable-subscriber profile.
 *
 * This mirrors the webhook profile shape while keeping the outbound token and
 * routing config declarative and host-resolved.
 *
 * @example `const mw: TelegramMiddleware = { kind: 'telegram', token: { ref: 'secret:telegram-bot' }, events: ['permission_request'] }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface TelegramMiddleware {
  /** Stable discriminator for telegram durable-subscriber middleware. */
  readonly kind: 'telegram'
  /** Optional human-readable profile name used in logs and diagnostics. */
  readonly name?: string
  /** Optional host-owned target alias for future routing extensions. */
  readonly target?: string
  /** Telegram bot token or host-resolved token reference. */
  readonly token?: string | DurableSubscriberSecretRef
  /** Optional Telegram chat/channel routing identifier. */
  readonly chatId?: string
  /** Optional allow-list of Telegram user ids allowed to resolve approvals. */
  readonly allowedUserIds?: readonly string[]
  /** Active approval scope supported by the current Rust TelegramSubscriber. */
  readonly scope?: 'tool_calls'
  /** Optional Telegram Bot API base URL. */
  readonly apiBaseUrl?: string
  /** Optional approval timeout in milliseconds. */
  readonly approvalTimeoutMs?: number
  /** Poll interval for Telegram update fetches in milliseconds. */
  readonly pollIntervalMs?: number
  /** Long-poll timeout for Telegram update fetches in milliseconds. */
  readonly pollTimeoutMs?: number
  /** Message rendering mode used for approval cards. */
  readonly parseMode?: 'html' | 'markdown_v2'
  /** Legacy placeholder selectors retained until the TS/Rust surfaces fully converge. */
  readonly events?: readonly DurableSubscriberEventSelector[]
  /** Legacy placeholder key strategy retained until the TS/Rust surfaces fully converge. */
  readonly keyBy?: Exclude<DurableSubscriberKeyStrategy, 'session'>
  /** Legacy placeholder retry policy retained until the TS/Rust surfaces fully converge. */
  readonly retry?: DurableSubscriberRetryPolicy
}

/**
 * Middleware spec for the `AutoApproveSubscriber` active profile.
 *
 * This profile shares the same canonical approval completion path as the
 * passive approval gate introduced earlier in the rollout.
 *
 * @example `const mw: AutoApproveMiddleware = { kind: 'autoApprove' }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface AutoApproveMiddleware {
  /** Stable discriminator for auto-approve durable-subscriber middleware. */
  readonly kind: 'autoApprove'
  /** Optional human-readable profile name used in logs and diagnostics. */
  readonly name?: string
  /** Optional event selectors; defaults to `permission_request`. */
  readonly events?: readonly DurableSubscriberEventSelector[]
  /** Optional bounded retry policy owned by the infrastructure plane. */
  readonly retry?: DurableSubscriberRetryPolicy
}

/**
 * Middleware spec for the peer-routing durable-subscriber profile.
 *
 * @example `const mw: PeerRoutingMiddleware = { kind: 'peerRouting' }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface PeerRoutingMiddleware {
  /** Stable discriminator for peer-routing durable-subscriber middleware. */
  readonly kind: 'peerRouting'
  /** Optional human-readable profile name used in logs and diagnostics. */
  readonly name?: string
}

/**
 * Middleware spec for the always-on deployment wake durable-subscriber profile.
 *
 * @example `const mw: WakeDeploymentMiddleware = { kind: 'wakeDeployment' }`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export interface WakeDeploymentMiddleware {
  /** Stable discriminator for deployment-wake durable-subscriber middleware. */
  readonly kind: 'wakeDeployment'
  /** Optional human-readable profile name used in logs and diagnostics. */
  readonly name?: string
}

/**
 * Union of declarative durable-subscriber profile configs exposed to TypeScript.
 *
 * @example `const profile: DurableSubscriberMiddleware = webhook({ target: 'slack-approvals', events: ['permission_request'] })`
 *
 * @remarks Anthropic primitive: Middleware.
 */
export type DurableSubscriberMiddleware =
  | WebhookMiddleware
  | TelegramMiddleware
  | AutoApproveMiddleware
  | PeerRoutingMiddleware
  | WakeDeploymentMiddleware

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
  | AttachToolsMiddleware
  | SecretsProxyMiddleware
  | DurableSubscriberMiddleware

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
 * Hosted Fireline transport that provisions a runtime before attaching ACP.
 *
 * @example `await conductor.connect_to({ kind: 'hosted', url: 'http://127.0.0.1:4440' })`
 *
 * @remarks Anthropic primitive: Conductor.
 */
export interface HostedTransport {
  /** Stable discriminator for hosted Fireline provisioning. */
  readonly kind: 'hosted'
  /** Base URL for the Fireline control plane. */
  readonly url: string
  /** Optional bearer token forwarded to provisioning requests. */
  readonly token?: string
  /** Optional runtime name override for this launch. */
  readonly name?: string
  /** Optional durable state stream name shared across launches. */
  readonly stateStream?: string
  /** Optional ACP client name used during initialize. */
  readonly clientName?: string
}

/**
 * Direct websocket transport terminating onto an already-running ACP endpoint.
 *
 * @example `await conductor.connect_to({ kind: 'websocket', url: handle.acp.url })`
 *
 * @remarks Anthropic primitive: Conductor.
 */
export interface WebSocketTransport {
  /** Stable discriminator for ACP websocket transport. */
  readonly kind: 'websocket'
  /** Absolute websocket URL for the ACP endpoint. */
  readonly url: string
  /** Optional static headers required when opening the websocket. */
  readonly headers?: Readonly<Record<string, string>>
  /** Optional ACP client name used during initialize. */
  readonly clientName?: string
}

/**
 * Native stdio transport that boots a Fireline child process and speaks ACP over stdin/stdout.
 *
 * @example `await conductor.connect_to({ kind: 'stdio', durableStreamsUrl: 'http://127.0.0.1:8787/v1/stream' })`
 *
 * @remarks Anthropic primitive: Conductor.
 */
export interface StdioTransport {
  /** Stable discriminator for native ACP stdio. */
  readonly kind: 'stdio'
  /** Optional explicit `fireline` binary path. Defaults to `process.env.FIRELINE_BIN ?? 'fireline'`. */
  readonly firelineBin?: string
  /** Durable-streams base URL required by the stdio child runtime. */
  readonly durableStreamsUrl?: string
  /** Optional bind host for the child runtime. Defaults to `127.0.0.1`. */
  readonly host?: string
  /** Optional bind port for the child runtime. Defaults to `0`. */
  readonly port?: number
  /** Optional runtime name override for this launch. */
  readonly name?: string
  /** Optional durable state stream name shared across launches. */
  readonly stateStream?: string
  /** Optional explicit peer-directory path forwarded to the child runtime. */
  readonly peerDirectoryPath?: string
  /** Optional child-process working directory. */
  readonly cwd?: string
  /** Optional environment variables merged into the child process env. */
  readonly env?: Readonly<Record<string, string>>
  /** Optional ACP client name used during initialize. */
  readonly clientName?: string
}

/**
 * Preconstructed ACP stream transport.
 *
 * @example `await conductor.connect_to({ kind: 'stream', stream })`
 *
 * @remarks Anthropic primitive: Conductor.
 */
export interface StreamTransport {
  /** Stable discriminator for an already-open ACP stream. */
  readonly kind: 'stream'
  /** ACP stream implementation consumed by the SDK connection constructor. */
  readonly stream: Stream
  /** Optional ACP client name used during initialize. */
  readonly clientName?: string
}

type ClientConductorTransport =
  | HostedTransport
  | WebSocketTransport
  | StdioTransport
  | StreamTransport

/**
 * Transport union accepted by `Conductor.connect_to(...)`.
 *
 * @remarks Anthropic primitive: Conductor.
 */
export type ConductorTransport<
  Role extends 'client' | 'agent' = 'client',
> = Role extends 'client' ? ClientConductorTransport : never

/**
 * Options accepted by `Harness.start()` and topology `start()` helpers.
 *
 * @example `await harness.start({ serverUrl: 'http://127.0.0.1:4440', name: 'demo' })`
 *
 * @remarks Anthropic primitive: Conductor.
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
 * Runnable conductor specification produced by `compose()`.
 *
 * @example `const spec: ConductorSpec<'default'> = compose(sandbox(), middleware([]), agent(['node', 'agent.mjs']))`
 *
 * @remarks Anthropic primitive: Conductor.
 */
interface ConductorSpecBase<Name extends string = string> {
  /** Logical conductor name used in stream names and future topologies. */
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

export interface ConductorSpec<Name extends string = string> extends ConductorSpecBase<Name> {
  /** Stable discriminator for serialized conductor configs. */
  readonly kind: 'conductor'
}

/**
 * Backwards-compatible alias for serialized harness specs.
 *
 * @deprecated Use `ConductorSpec` instead.
 *
 * @remarks Anthropic primitive: Conductor.
 */
export interface HarnessSpec<Name extends string = string> extends ConductorSpecBase<Name> {
  /** Stable discriminator retained for the migration window. */
  readonly kind: 'harness'
}

/**
 * Public alias for the config accepted by `Sandbox.provision()`.
 *
 * @example `const config: SandboxConfig = compose(sandbox(), middleware([trace()]), agent(['node', 'agent.mjs']))`
 *
 * @remarks Anthropic primitive: Conductor.
 */
export type SandboxConfig<Name extends string = string> =
  | ConductorSpec<Name>
  | HarnessSpec<Name>

/**
 * Backwards-compatible alias for serialized harness specs.
 *
 * @deprecated Use `ConductorSpec` instead.
 *
 * @remarks Anthropic primitive: Conductor.
 */
export type HarnessConfig<Name extends string = string> = SandboxConfig<Name>

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
 * Conductor-scoped handle returned by `Conductor.start()`.
 *
 * @example `const handle = await conductor.start({ serverUrl })`
 *
 * @remarks Anthropic primitive: Conductor.
 */
export interface HarnessHandle<Name extends string = string> extends SandboxHandle {
  /** Logical conductor name used when the handle was launched. */
  readonly name: Name
}

/**
 * Runnable conductor value created by `compose()`.
 *
 * @example `const reviewer = compose(sandbox(), middleware([trace()]), agent(['agent'])).as('reviewer')`
 *
 * @remarks Anthropic primitive: Conductor.
 */
export interface Conductor<
  Name extends string = string,
  Role extends 'client' | 'agent' = 'client',
> extends ConductorSpec<Name> {
  /** Role-direction marker reserved for future agent-facing proxy compositions. */
  readonly role: Role
  /** Returns a renamed conductor while preserving sandbox, middleware, and agent config. */
  as<NextName extends string>(name: NextName): Conductor<NextName, Role>
  /** Returns a role-cast conductor for forward-compatible proxy-chain typing. */
  asRole<NextRole extends 'client' | 'agent'>(
    role: NextRole,
  ): Conductor<Name, NextRole>
  /** Terminates the conductor onto the supplied transport and returns a live ACP connection. */
  connect_to(transport: ConductorTransport<Role>): Promise<Role extends 'client' ? ConnectedAcp : never>
  /**
   * Provisions the conductor and returns a live Fireline agent handle.
   *
   * @deprecated Prefer `connect_to({ kind: 'hosted', ... })` for the one-call transport shape.
   */
  start(options: StartOptions): Promise<FirelineAgent<Name>>
}

/**
 * Backwards-compatible alias for the old Fireline-local harness name.
 *
 * @deprecated Use `Conductor` instead.
 *
 * @remarks Anthropic primitive: Conductor.
 */
export type Harness<Name extends string = string> = Conductor<Name, 'client'>

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
