export interface TopologyComponentSpec {
  name: string
  config?: Record<string, unknown>
}

export interface TopologySpec {
  components: TopologyComponentSpec[]
}

export type ContextPlacement = 'prepend' | 'append'

export type ContextSourceSpec =
  | { kind: 'datetime' }
  | { kind: 'workspaceFile'; path: string }
  | { kind: 'staticText'; text: string }
