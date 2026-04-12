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

  async get(_id: string): Promise<SandboxDescriptor | null> {
    throw new Error('SandboxAdmin.get() is wired in phase 2')
  }

  async list(_labels?: Readonly<Record<string, string>>): Promise<SandboxDescriptor[]> {
    throw new Error('SandboxAdmin.list() is wired in phase 2')
  }

  async destroy(_id: string): Promise<SandboxDescriptor | null> {
    throw new Error('SandboxAdmin.destroy() is wired in phase 2')
  }

  async status(_id: string): Promise<SandboxStatus | null> {
    throw new Error('SandboxAdmin.status() is wired in phase 2')
  }

  async healthCheck(): Promise<boolean> {
    throw new Error('SandboxAdmin.healthCheck() is wired in phase 2')
  }
}
