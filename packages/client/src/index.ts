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
  type Endpoint,
  type HostClient,
  type HostClientOptions,
  type ResourceRef,
  type RuntimeDescriptor,
  type RuntimeProviderKind,
  type RuntimeProviderRequest,
  type RuntimeStatus,
} from './host.js'
import {
  ACP_AGENT_REGISTRY_URL,
  createCatalogClient,
  resolveAgentLaunch,
  type AgentCatalogEntry,
  type AgentCatalogSource,
  type AgentDistribution,
  type AgentDistributionKind,
  type BinaryDistribution,
  type BinaryTarget,
  type CatalogAgentLaunchSpec,
  type CatalogArch,
  type CatalogClient,
  type CatalogClientOptions,
  type CatalogPlatform,
  type CommandDistribution,
  type ManualAgentLaunchSpec,
  type NpxDistribution,
  type ResolveAgentOptions,
  type ResolvedAgentLaunch,
  type RuntimeAgentSpec,
  type UvxDistribution,
} from './catalog.js'
import {
  createTopologyBuilder,
  type AuditTopologyConfig,
  type ContextPlacement,
  type ContextInjectionTopologyConfig,
  type ContextSourceSpec,
  type TopologyBuilder,
  type TopologyComponentSpec,
  type TopologySpec,
} from './topology.js'

export type {
  AcpConnectOptions,
  AcpInitializeOptions,
  CreateRuntimeSpec,
  Endpoint,
  AgentCatalogEntry,
  AgentCatalogSource,
  AgentDistribution,
  AgentDistributionKind,
  HostClient,
  HostClientOptions,
  OpenAcpConnection,
  ResourceRef,
  BinaryDistribution,
  BinaryTarget,
  CatalogAgentLaunchSpec,
  CatalogArch,
  CatalogClient,
  CatalogClientOptions,
  CatalogPlatform,
  CommandDistribution,
  ManualAgentLaunchSpec,
  NpxDistribution,
  ResolveAgentOptions,
  ResolvedAgentLaunch,
  RuntimeDescriptor,
  RuntimeAgentSpec,
  RuntimeProviderKind,
  RuntimeProviderRequest,
  RuntimeStatus,
  UvxDistribution,
  AuditTopologyConfig,
  ContextPlacement,
  ContextInjectionTopologyConfig,
  ContextSourceSpec,
  TopologyBuilder,
  TopologyComponentSpec,
  TopologySpec,
}

export interface FirelineClient {
  acp: {
    connect(options: AcpConnectOptions): Promise<OpenAcpConnection>
  }
  host: HostClient
  catalog: CatalogClient
  topology: {
    builder(): TopologyBuilder
  }
  state: {
    open(config: FirelineDBConfig): FirelineDB
  }
  close(): Promise<void>
}

export interface FirelineClientOptions {
  host?: HostClientOptions
  catalog?: CatalogClientOptions
}

export function createFirelineClient(options: FirelineClientOptions = {}): FirelineClient {
  const host = createHostClient({
    ...options.host,
    catalog: options.catalog ?? options.host?.catalog,
  })
  const catalog = createCatalogClient(options.catalog)
  return {
    acp: {
      connect(options) {
        return connectAcp(options)
      },
    },
    host,
    catalog,
    topology: {
      builder() {
        return createTopologyBuilder()
      },
    },
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

export {
  ACP_AGENT_REGISTRY_URL,
  connectAcp,
  createCatalogClient,
  createHostClient,
  defaultRuntimeRegistryPath,
  resolveAgentLaunch,
  createTopologyBuilder,
}
