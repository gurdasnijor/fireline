import { createFirelineDB, type FirelineDB, type FirelineDBConfig } from '@fireline/state'

import {
  connectAcp,
  type AcpConnectOptions,
  type AcpInitializeOptions,
  type OpenAcpConnection,
} from './acp.browser.js'
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
  OpenAcpConnection,
  AuditTopologyConfig,
  ContextPlacement,
  ContextInjectionTopologyConfig,
  ContextSourceSpec,
  TopologyBuilder,
  TopologyComponentSpec,
  TopologySpec,
}

export interface BrowserFirelineClient {
  acp: {
    connect(options: AcpConnectOptions): Promise<OpenAcpConnection>
  }
  topology: {
    builder(): TopologyBuilder
  }
  state: {
    open(config: FirelineDBConfig): FirelineDB
  }
  close(): Promise<void>
}

export function createBrowserFirelineClient(): BrowserFirelineClient {
  return {
    acp: {
      connect(options) {
        return connectAcp(options)
      },
    },
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
    async close() {
      // Browser clients do not own background processes.
    },
  }
}

export { connectAcp, createTopologyBuilder }
