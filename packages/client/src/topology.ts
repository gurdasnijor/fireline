export interface TopologyComponentSpec {
  name: string
  config?: Record<string, unknown>
}

export interface TopologySpec {
  components: TopologyComponentSpec[]
}

export interface AuditTopologyConfig {
  streamName: string
  includeMethods?: string[]
}

export type ContextPlacement = 'prepend' | 'append'

export type ContextSourceSpec =
  | { kind: 'datetime' }
  | { kind: 'workspaceFile'; path: string }
  | { kind: 'staticText'; text: string }

export interface ContextInjectionTopologyConfig {
  prependText?: string
  placement?: ContextPlacement
  sources?: ContextSourceSpec[]
}

export class TopologyBuilder {
  private readonly components: TopologyComponentSpec[] = []

  audit(config: AuditTopologyConfig): this {
    this.components.push({
      name: 'audit',
      config: { ...config },
    })
    return this
  }

  contextInjection(config: ContextInjectionTopologyConfig): this {
    this.components.push({
      name: 'context_injection',
      config: { ...config },
    })
    return this
  }

  peerMcp(): this {
    this.components.push({
      name: 'peer_mcp',
    })
    return this
  }

  build(): TopologySpec {
    return {
      components: [...this.components],
    }
  }
}

export function createTopologyBuilder(): TopologyBuilder {
  return new TopologyBuilder()
}
