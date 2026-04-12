import { SandboxAdmin } from './admin.js'
import { connectAcp, type ConnectedAcp } from './connect.js'
import { appendApprovalResolved } from './events.js'
import type { HarnessHandle, SandboxHandle } from './types.js'

/**
 * Outcome payload for externally resolving a Fireline approval request.
 */
export interface ResolvePermissionOutcome {
  readonly allow: boolean
  readonly resolvedBy?: string
}

interface FirelineAgentOptions<Name extends string> {
  readonly serverUrl: string
  readonly token?: string
  readonly name: Name
  readonly handle: SandboxHandle
}

/**
 * Live Fireline agent handle.
 *
 * Preserves the structural sandbox-handle fields while adding imperative
 * methods for ACP connection, approval resolution, and lifecycle operations.
 */
export class FirelineAgent<Name extends string = string> implements HarnessHandle<Name> {
  readonly id: string
  readonly provider: string
  readonly acp: SandboxHandle['acp']
  readonly state: SandboxHandle['state']
  readonly name: Name

  readonly #admin: SandboxAdmin

  constructor(options: FirelineAgentOptions<Name>) {
    this.id = options.handle.id
    this.provider = options.handle.provider
    this.acp = options.handle.acp
    this.state = options.handle.state
    this.name = options.name
    this.#admin = new SandboxAdmin({
      serverUrl: options.serverUrl,
      token: options.token,
    })
  }

  connect(clientName?: string): Promise<ConnectedAcp> {
    return connectAcp(this.acp, clientName)
  }

  async resolvePermission(
    sessionId: string,
    requestId: string,
    outcome: ResolvePermissionOutcome,
  ): Promise<void> {
    await appendApprovalResolved({
      streamUrl: this.state.url,
      sessionId,
      requestId,
      allow: outcome.allow,
      resolvedBy: outcome.resolvedBy,
    })
  }

  async stop(): Promise<void> {
    await this.#admin.destroy(this.id)
  }

  async destroy(): Promise<void> {
    await this.#admin.destroy(this.id)
  }
}
