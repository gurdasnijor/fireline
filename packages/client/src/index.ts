import { createFirelineDB, type FirelineDB, type FirelineDBConfig } from '@fireline/state'

import {
  connectAcp,
  type AcpConnectOptions,
  type AcpInitializeOptions,
  type OpenAcpConnection,
} from './acp.js'
import {
  createHostClient,
  defaultRuntimeRegistryPath,
  type CreateRuntimeSpec,
  type HostClient,
  type HostClientOptions,
  type RuntimeDescriptor,
  type RuntimeProviderKind,
  type RuntimeProviderRequest,
  type RuntimeStatus,
} from './host.js'

export type {
  AcpConnectOptions,
  AcpInitializeOptions,
  CreateRuntimeSpec,
  HostClient,
  HostClientOptions,
  OpenAcpConnection,
  RuntimeDescriptor,
  RuntimeProviderKind,
  RuntimeProviderRequest,
  RuntimeStatus,
}

export interface FirelineClient {
  acp: {
    connect(options: AcpConnectOptions): Promise<OpenAcpConnection>
  }
  host: HostClient
  state: {
    open(config: FirelineDBConfig): FirelineDB
  }
  close(): Promise<void>
}

export interface FirelineClientOptions {
  host?: HostClientOptions
}

export function createFirelineClient(options: FirelineClientOptions = {}): FirelineClient {
  const host = createHostClient(options.host)
  return {
    acp: {
      connect(options) {
        return connectAcp(options)
      },
    },
    host,
    state: {
      open(config) {
        return createFirelineDB(config)
      },
    },
    close() {
      return host.close()
    },
  }
}

export { connectAcp, createHostClient, defaultRuntimeRegistryPath }
