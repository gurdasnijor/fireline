import { requestControlPlane } from './control-plane.js'
import type { SandboxDescriptor, SandboxStatus } from './types.js'

/**
 * Connection settings for the sandbox admin client.
 *
 * @example `const admin = new SandboxAdmin({ serverUrl: 'http://127.0.0.1:4440' })`
 *
 * @remarks Anthropic primitive: Sandbox.
 */
export interface SandboxAdminOptions {
  /** Base URL for the Fireline host or control plane. */
  readonly serverUrl: string
  /** Optional bearer token forwarded to admin requests. */
  readonly token?: string
}

/**
 * Structural contract for the sandbox admin surface.
 *
 * @example `const sandboxes = await admin.list()`
 *
 * @remarks Anthropic primitive: Sandbox.
 */
export interface SandboxAdmin {
  /**
   * Reads one sandbox descriptor by id.
   *
   * @example `const descriptor = await admin.get('sandbox-1')`
   *
   * @remarks Anthropic primitive: Sandbox.
   */
  get(id: string): Promise<SandboxDescriptor | null>

  /**
   * Lists known sandboxes, optionally filtered by labels client-side.
   *
   * @example `const sandboxes = await admin.list({ team: 'infra' })`
   *
   * @remarks Anthropic primitive: Sandbox.
   */
  list(labels?: Readonly<Record<string, string>>): Promise<SandboxDescriptor[]>

  /**
   * Lists known sandboxes through the explicit infrastructure-plane surface.
   *
   * @example `const sandboxes = await admin.listSandboxes()`
   *
   * @remarks Anthropic primitive: Sandbox.
   */
  listSandboxes(labels?: Readonly<Record<string, string>>): Promise<SandboxDescriptor[]>

  /**
   * Lists hosts through the infrastructure-plane admin surface.
   *
   * @example `const hosts = await admin.listHosts()`
   *
   * @remarks Anthropic primitive: Host.
   */
  listHosts(labels?: Readonly<Record<string, string>>): Promise<SandboxDescriptor[]>

  /**
   * Destroys a sandbox by id.
   *
   * @example `await admin.destroy('sandbox-1')`
   *
   * @remarks Anthropic primitive: Sandbox.
   */
  destroy(id: string): Promise<SandboxDescriptor | null>

  /**
   * Reads only the status of a sandbox by id.
   *
   * @example `const status = await admin.status('sandbox-1')`
   *
   * @remarks Anthropic primitive: Sandbox.
   */
  status(id: string): Promise<SandboxStatus | null>

  /**
   * Checks whether the Fireline host health endpoint is reachable.
   *
   * @example `const ok = await admin.healthCheck()`
   *
   * @remarks Anthropic primitive: Sandbox.
   */
  healthCheck(): Promise<boolean>
}

/**
 * Concrete client for sandbox admin reads and destructive lifecycle operations.
 *
 * @example `const admin = new SandboxAdmin({ serverUrl })`
 *
 * @remarks Anthropic primitive: Sandbox.
 */
export class SandboxAdmin {
  /** Base URL for the Fireline host or control plane. */
  readonly serverUrl: string
  /** Optional bearer token forwarded to admin requests. */
  readonly token?: string

  constructor(options: SandboxAdminOptions) {
    this.serverUrl = options.serverUrl
    this.token = options.token
  }

  /**
   * Reads one sandbox descriptor by id.
   *
   * @example `const descriptor = await admin.get('sandbox-1')`
   *
   * @remarks Anthropic primitive: Sandbox.
   */
  async get(id: string): Promise<SandboxDescriptor | null> {
    return requestControlPlane<SandboxDescriptor>(
      this,
      `/v1/sandboxes/${encodeURIComponent(id)}`,
      { allowNotFound: true },
    )
  }

  /**
   * Lists known sandboxes, optionally filtered by labels client-side.
   *
   * @example `const sandboxes = await admin.list({ team: 'infra' })`
   *
   * @remarks Anthropic primitive: Sandbox.
   */
  async list(labels?: Readonly<Record<string, string>>): Promise<SandboxDescriptor[]> {
    const sandboxes =
      (await requestControlPlane<SandboxDescriptor[]>(this, '/v1/sandboxes')) ?? []
    if (!labels || Object.keys(labels).length === 0) {
      return sandboxes
    }
    return sandboxes.filter((descriptor) => matchesLabels(descriptor.labels, labels))
  }

  /**
   * Lists known sandboxes through the explicit infrastructure-plane surface.
   *
   * @example `const sandboxes = await admin.listSandboxes()`
   *
   * @remarks Anthropic primitive: Sandbox.
   */
  async listSandboxes(labels?: Readonly<Record<string, string>>): Promise<SandboxDescriptor[]> {
    return this.list(labels)
  }

  /**
   * Lists hosts through the infrastructure-plane admin surface.
   *
   * @example `const hosts = await admin.listHosts()`
   *
   * @remarks Anthropic primitive: Host.
   */
  async listHosts(labels?: Readonly<Record<string, string>>): Promise<SandboxDescriptor[]> {
    return this.list(labels)
  }

  /**
   * Destroys a sandbox by id.
   *
   * @example `await admin.destroy('sandbox-1')`
   *
   * @remarks Anthropic primitive: Sandbox.
   */
  async destroy(id: string): Promise<SandboxDescriptor | null> {
    return requestControlPlane<SandboxDescriptor>(
      this,
      `/v1/sandboxes/${encodeURIComponent(id)}`,
      {
        method: 'DELETE',
        allowNotFound: true,
      },
    )
  }

  /**
   * Reads only the status of a sandbox by id.
   *
   * @example `const status = await admin.status('sandbox-1')`
   *
   * @remarks Anthropic primitive: Sandbox.
   */
  async status(id: string): Promise<SandboxStatus | null> {
    return (await this.get(id))?.status ?? null
  }

  /**
   * Checks whether the Fireline host health endpoint is reachable.
   *
   * @example `const ok = await admin.healthCheck()`
   *
   * @remarks Anthropic primitive: Sandbox.
   */
  async healthCheck(): Promise<boolean> {
    const response = await fetch(`${this.serverUrl.replace(/\/$/, '')}/healthz`, {
      headers: {
        accept: 'text/plain',
        ...(this.token ? { authorization: `Bearer ${this.token}` } : {}),
      },
    })
    return response.ok
  }
}

function matchesLabels(
  actual: Readonly<Record<string, string>>,
  expected: Readonly<Record<string, string>>,
): boolean {
  return Object.entries(expected).every(([key, value]) => actual[key] === value)
}
