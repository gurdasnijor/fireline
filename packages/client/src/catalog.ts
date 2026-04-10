import { arch as osArch, platform as osPlatform } from 'node:os'

export const ACP_AGENT_REGISTRY_URL =
  'https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json'

export type AgentCatalogSource = 'registry' | 'local'
export type AgentDistributionKind = 'command' | 'npx' | 'uvx' | 'binary'
export type CatalogPlatform = 'darwin' | 'linux' | 'windows'
export type CatalogArch = 'x86_64' | 'aarch64'

export interface CommandDistribution {
  kind: 'command'
  command: string[]
}

export interface NpxDistribution {
  kind: 'npx'
  package: string
  args?: string[]
}

export interface UvxDistribution {
  kind: 'uvx'
  package: string
  args?: string[]
}

export interface BinaryTarget {
  target: `${CatalogPlatform}-${CatalogArch}`
  archive: string
  cmd: string
  args?: string[]
}

export interface BinaryDistribution {
  kind: 'binary'
  targets: BinaryTarget[]
}

export type AgentDistribution =
  | CommandDistribution
  | NpxDistribution
  | UvxDistribution
  | BinaryDistribution

export interface AgentCatalogEntry {
  source: AgentCatalogSource
  id: string
  name: string
  version: string
  description?: string
  website?: string
  repository?: string
  icon?: string
  authors?: string[]
  license?: string
  distributions: AgentDistribution[]
}

export interface CatalogAgentLaunchSpec {
  source: 'catalog'
  agentId: string
  preferredKinds?: AgentDistributionKind[]
}

export interface ManualAgentLaunchSpec {
  source: 'manual'
  command: string[]
}

export type RuntimeAgentSpec = CatalogAgentLaunchSpec | ManualAgentLaunchSpec

export interface ResolvedAgentLaunch {
  agentId: string
  source: AgentCatalogSource | 'manual'
  distributionKind: Exclude<AgentDistributionKind, 'binary'>
  command: string[]
  version?: string
}

export interface ResolveAgentOptions {
  preferredKinds?: AgentDistributionKind[]
  platform?: CatalogPlatform
  arch?: CatalogArch
}

export interface CatalogClientOptions {
  registryUrl?: string
  localEntries?: AgentCatalogEntry[]
  fetchImpl?: typeof fetch
}

export interface CatalogClient {
  listAgents(): Promise<AgentCatalogEntry[]>
  getAgent(agentId: string): Promise<AgentCatalogEntry | null>
  resolveAgent(agentId: string, options?: ResolveAgentOptions): Promise<ResolvedAgentLaunch>
}

interface RegistryDistributionBinaryTarget {
  archive: string
  cmd: string
  args?: string[]
}

interface RegistryDistribution {
  binary?: Record<string, RegistryDistributionBinaryTarget>
  npx?: {
    package: string
    args?: string[]
  }
  uvx?: {
    package: string
    args?: string[]
  }
}

interface RegistryAgentEntry {
  id: string
  name: string
  version: string
  description?: string
  website?: string
  repository?: string
  icon?: string
  authors?: string[]
  license?: string
  distribution?: RegistryDistribution
}

interface RegistryResponse {
  agents?: RegistryAgentEntry[]
}

export function createCatalogClient(options: CatalogClientOptions = {}): CatalogClient {
  const registryUrl = options.registryUrl ?? ACP_AGENT_REGISTRY_URL
  const fetchImpl = options.fetchImpl ?? fetch
  const localEntries = [...(options.localEntries ?? [])]

  let cachedRegistry: Promise<AgentCatalogEntry[]> | null = null

  async function loadRegistryEntries(): Promise<AgentCatalogEntry[]> {
    if (!cachedRegistry) {
      cachedRegistry = (async () => {
        const response = await fetchImpl(registryUrl, {
          headers: {
            'user-agent': 'fireline-client',
            accept: 'application/json',
          },
        })
        if (!response.ok) {
          throw new Error(`ACP registry fetch failed: ${response.status} ${response.statusText}`)
        }
        const payload = (await response.json()) as RegistryResponse
        return (payload.agents ?? []).map(normalizeRegistryAgent)
      })()
    }
    return cachedRegistry
  }

  return {
    async listAgents() {
      const registryEntries = await loadRegistryEntries()
      return mergeAgentEntries(localEntries, registryEntries)
    },

    async getAgent(agentId) {
      const agents = await this.listAgents()
      return agents.find((agent) => agent.id === agentId) ?? null
    },

    async resolveAgent(agentId, resolveOptions) {
      const entry = await this.getAgent(agentId)
      if (!entry) {
        throw new Error(`unknown agent '${agentId}'`)
      }
      return resolveAgentLaunch(entry, resolveOptions)
    },
  }
}

export function resolveAgentLaunch(
  agent: AgentCatalogEntry,
  options: ResolveAgentOptions = {},
): ResolvedAgentLaunch {
  const platform = options.platform ?? currentPlatform()
  const arch = options.arch ?? currentArch()
  const preferredKinds = options.preferredKinds ?? ['command', 'npx', 'uvx', 'binary']

  for (const kind of preferredKinds) {
    switch (kind) {
      case 'command': {
        const dist = agent.distributions.find(
          (distribution): distribution is CommandDistribution => distribution.kind === 'command',
        )
        if (dist) {
          return {
            agentId: agent.id,
            source: agent.source,
            distributionKind: 'command',
            command: [...dist.command],
            version: agent.version,
          }
        }
        break
      }
      case 'npx': {
        const dist = agent.distributions.find(
          (distribution): distribution is NpxDistribution => distribution.kind === 'npx',
        )
        if (dist) {
          return {
            agentId: agent.id,
            source: agent.source,
            distributionKind: 'npx',
            command: ['npx', '-y', dist.package, ...(dist.args ?? [])],
            version: agent.version,
          }
        }
        break
      }
      case 'uvx': {
        const dist = agent.distributions.find(
          (distribution): distribution is UvxDistribution => distribution.kind === 'uvx',
        )
        if (dist) {
          return {
            agentId: agent.id,
            source: agent.source,
            distributionKind: 'uvx',
            command: ['uvx', dist.package, ...(dist.args ?? [])],
            version: agent.version,
          }
        }
        break
      }
      case 'binary': {
        const dist = agent.distributions.find(
          (distribution): distribution is BinaryDistribution => distribution.kind === 'binary',
        )
        if (dist) {
          const target = `${platform}-${arch}`
          const match = dist.targets.find((candidate) => candidate.target === target)
          if (match) {
            throw new Error(
              `agent '${agent.id}' only resolves via binary archive for ${target}; local binary installation is not implemented yet`,
            )
          }
        }
        break
      }
    }
  }

  throw new Error(
    `agent '${agent.id}' has no supported distribution for this runtime; available kinds: ${agent.distributions.map((distribution) => distribution.kind).join(', ') || 'none'}`,
  )
}

function normalizeRegistryAgent(entry: RegistryAgentEntry): AgentCatalogEntry {
  const distributions: AgentDistribution[] = []
  const distribution = entry.distribution ?? {}

  if (distribution.npx?.package) {
    distributions.push({
      kind: 'npx',
      package: distribution.npx.package,
      args: distribution.npx.args,
    })
  }

  if (distribution.uvx?.package) {
    distributions.push({
      kind: 'uvx',
      package: distribution.uvx.package,
      args: distribution.uvx.args,
    })
  }

  if (distribution.binary) {
    const targets = Object.entries(distribution.binary)
      .filter((entry): entry is [BinaryTarget['target'], RegistryDistributionBinaryTarget] =>
        isCatalogTarget(entry[0]),
      )
      .map(([target, binary]) => ({
        target,
        archive: binary.archive,
        cmd: binary.cmd,
        args: binary.args,
      }))

    if (targets.length > 0) {
      distributions.push({
        kind: 'binary',
        targets,
      })
    }
  }

  return {
    source: 'registry',
    id: entry.id,
    name: entry.name,
    version: entry.version,
    description: entry.description,
    website: entry.website,
    repository: entry.repository,
    icon: entry.icon,
    authors: entry.authors,
    license: entry.license,
    distributions,
  }
}

function mergeAgentEntries(
  localEntries: AgentCatalogEntry[],
  registryEntries: AgentCatalogEntry[],
): AgentCatalogEntry[] {
  const merged = new Map<string, AgentCatalogEntry>()
  for (const entry of registryEntries) {
    merged.set(entry.id, entry)
  }
  for (const entry of localEntries) {
    merged.set(entry.id, entry)
  }
  return [...merged.values()].sort((left, right) => left.name.localeCompare(right.name))
}

function currentPlatform(): CatalogPlatform {
  switch (osPlatform()) {
    case 'darwin':
      return 'darwin'
    case 'win32':
      return 'windows'
    default:
      return 'linux'
  }
}

function currentArch(): CatalogArch {
  switch (osArch()) {
    case 'arm64':
      return 'aarch64'
    default:
      return 'x86_64'
  }
}

function isCatalogTarget(value: string): value is `${CatalogPlatform}-${CatalogArch}` {
  return /^(darwin|linux|windows)-(x86_64|aarch64)$/.test(value)
}
