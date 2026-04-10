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
import {
  createPeerClient,
  defaultPeerDirectoryPath,
  type PeerCallRequest,
  type PeerCallResult,
  type PeerClient,
  type PeerClientOptions,
  type PeerDescriptor,
  type PeerParentLineage,
} from './peer.js'

export type {
  AcpConnectOptions,
  AcpInitializeOptions,
  CreateRuntimeSpec,
  HostClient,
  HostClientOptions,
  OpenAcpConnection,
  PeerCallRequest,
  PeerCallResult,
  PeerClient,
  PeerClientOptions,
  PeerDescriptor,
  PeerParentLineage,
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
  peer: PeerClient
  state: {
    open(config: FirelineDBConfig): FirelineDB
  }
  close(): Promise<void>
}

export interface FirelineClientOptions {
  host?: HostClientOptions
  peer?: PeerClientOptions
}

export function createFirelineClient(options: FirelineClientOptions = {}): FirelineClient {
  const host = createHostClient(options.host)
  const peer = createPeerClient(options.peer)
  return {
    acp: {
      connect(options) {
        return connectAcp(options)
      },
    },
    host,
    peer,
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

export { connectAcp, createHostClient, createPeerClient, defaultPeerDirectoryPath, defaultRuntimeRegistryPath }
