import { db } from './db.js'
import { FirelineAgent } from './agent.js'
import { appendApprovalResolved } from './events.js'
import { connectAcp } from './connect.js'
import { Sandbox, agent, compose, middleware, sandbox } from './sandbox.js'
import { fanout, peer, pipe } from './topology.js'

const fireline = {
  db,
  compose,
  agent,
  sandbox,
  middleware,
  peer,
  fanout,
  pipe,
  FirelineAgent,
  connectAcp,
  appendApprovalResolved,
}

export default fireline

export { db }
export { FirelineAgent }
export { Sandbox, agent, compose, middleware, sandbox }
export { fanout, peer, pipe }
export { appendApprovalResolved } from './events.js'
export { connectAcp } from './connect.js'
export type {
  SessionId,
  RequestId,
  ToolCallId,
  PromptRequestRef,
  ToolInvocationRef,
} from './acp-ids.js'

export type { FirelineDB, FirelineDbOptions } from './db.js'
export type { ResolvePermissionOutcome } from './agent.js'
export type { SandboxClientOptions } from './sandbox.js'
export type { ConnectedAcp } from './connect.js'
export type {
  AgentConfig,
  AutoApproveMiddleware,
  AttachToolsMiddleware,
  CapabilityRef,
  CredentialRef,
  DurableSubscriberEventSelector,
  DurableSubscriberKeyStrategy,
  DurableSubscriberMiddleware,
  DurableSubscriberRetryPolicy,
  DurableSubscriberSecretRef,
  Endpoint,
  Harness,
  HarnessConfig,
  HarnessHandle,
  HarnessSpec,
  Middleware,
  MiddlewareChain,
  SandboxConfig,
  SandboxDefinition,
  SandboxDescriptor,
  SandboxHandle,
  SandboxProviderConfig,
  SandboxStatus,
  SecretBinding,
  SecretsProxyMiddleware,
  StartOptions,
  TelegramMiddleware,
  ToolAttachment,
  ToolDescriptor,
  TransportRef,
  WebhookMiddleware,
} from './types.js'
