export interface FirelineCollections {
  connections: object
  promptTurns: object
  pendingRequests: object
  permissions: object
  terminals: object
  runtimeInstances: object
  sessions: object
  childSessionEdges: object
  chunks: object
}

export interface FirelineDB {
  collections: FirelineCollections
  preload(): Promise<void>
  close(): void
}

export interface FirelineDBConfig {
  stateStreamUrl: string
  headers?: Record<string, string>
  signal?: AbortSignal
}

export function createFirelineDB(_config: FirelineDBConfig): FirelineDB {
  return {
    collections: {
      connections: {},
      promptTurns: {},
      pendingRequests: {},
      permissions: {},
      terminals: {},
      runtimeInstances: {},
      sessions: {},
      childSessionEdges: {},
      chunks: {},
    },
    async preload() {},
    close() {},
  }
}
