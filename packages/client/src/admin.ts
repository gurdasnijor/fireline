import { requestControlPlane } from './control-plane.js'
import type { SandboxDescriptor, SandboxStatus } from './types.js'

export interface SandboxAdminOptions {
  readonly serverUrl: string
  readonly token?: string
}

export interface SandboxAdmin {
  get(id: string): Promise<SandboxDescriptor | null>
  list(labels?: Readonly<Record<string, string>>): Promise<SandboxDescriptor[]>
  destroy(id: string): Promise<SandboxDescriptor | null>
  status(id: string): Promise<SandboxStatus | null>
  healthCheck(): Promise<boolean>
}

export class SandboxAdmin {
  readonly serverUrl: string
  readonly token?: string

  constructor(options: SandboxAdminOptions) {
    this.serverUrl = options.serverUrl
    this.token = options.token
  }

  async get(id: string): Promise<SandboxDescriptor | null> {
    return requestControlPlane<SandboxDescriptor>(
      this,
      `/v1/sandboxes/${encodeURIComponent(id)}`,
      { allowNotFound: true },
    )
  }

  async list(labels?: Readonly<Record<string, string>>): Promise<SandboxDescriptor[]> {
    const sandboxes =
      (await requestControlPlane<SandboxDescriptor[]>(this, '/v1/sandboxes')) ?? []
    if (!labels || Object.keys(labels).length === 0) {
      return sandboxes
    }
    return sandboxes.filter((descriptor) => matchesLabels(descriptor.labels, labels))
  }

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

  async status(id: string): Promise<SandboxStatus | null> {
    return (await this.get(id))?.status ?? null
  }

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
