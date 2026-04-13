import { realpath } from 'node:fs/promises'
import { isAbsolute, resolve as resolvePath } from 'node:path'

type JsonRecord = Record<string, unknown>

export type LoweredProvisionRequest = {
  readonly name: string
  readonly agentCommand: readonly string[]
  readonly topology: unknown
  readonly resources?: readonly unknown[]
  readonly envVars?: Readonly<Record<string, string>>
  readonly labels?: Readonly<Record<string, string>>
  readonly provider?: string
  readonly image?: string
  readonly model?: string
  readonly stateStream?: string
}

type MountedResource = {
  readonly host_path: string
  readonly mount_path: string
  readonly read_only: boolean
}

type BuildDirectHostArgsOptions = {
  readonly cwd?: string
  readonly env?: NodeJS.ProcessEnv
}

export function validateLoweredSpec(spec: LoweredProvisionRequest): void {
  if (spec.provider && spec.provider !== 'local') {
    throw new Error(
      `embedded-spec boot only supports local/direct-host lowering today; got provider='${spec.provider}'`,
    )
  }
  if (spec.image) {
    throw new Error('embedded-spec boot does not support docker image overrides')
  }
  if (spec.model) {
    throw new Error('embedded-spec boot does not support anthropic model lowering')
  }
  if (spec.envVars && Object.keys(spec.envVars).length > 0) {
    throw new Error('embedded-spec boot does not support sandbox env vars')
  }
  if (spec.labels && Object.keys(spec.labels).length > 0) {
    throw new Error('embedded-spec boot does not support sandbox labels')
  }
  if (!Array.isArray(spec.agentCommand) || spec.agentCommand.length === 0) {
    throw new Error('embedded-spec boot requires a non-empty agent command')
  }
}

export async function buildDirectHostArgs(
  spec: LoweredProvisionRequest,
  options: BuildDirectHostArgsOptions = {},
): Promise<string[]> {
  const env = options.env ?? process.env
  const cwd = options.cwd ?? process.cwd()
  const host = env.FIRELINE_HOST ?? '0.0.0.0'
  const port = env.FIRELINE_PORT ?? '4440'
  const durableStreamsUrl = env.FIRELINE_DURABLE_STREAMS_URL
  if (!durableStreamsUrl) {
    throw new Error('FIRELINE_DURABLE_STREAMS_URL must be set for embedded-spec boot')
  }
  const advertisedStateStreamUrl = env.FIRELINE_ADVERTISED_STATE_STREAM_URL
    ?? (spec.stateStream ? `${durableStreamsUrl.replace(/\/+$/, '')}/${spec.stateStream}` : null)
  const mountedResources = await resolveMountedResources(spec.resources ?? [], cwd)

  const args = [
    '--host', host,
    '--port', port,
    '--name', spec.name,
    '--durable-streams-url', durableStreamsUrl,
  ]

  if (spec.stateStream) {
    args.push('--state-stream', spec.stateStream)
  }

  if (advertisedStateStreamUrl) {
    args.push('--advertised-state-stream-url', advertisedStateStreamUrl)
  }

  if (hasTopologyComponents(spec.topology)) {
    args.push('--topology-json', JSON.stringify(spec.topology))
  }

  if (mountedResources.length > 0) {
    args.push('--mounted-resources-json', JSON.stringify(mountedResources))
  }

  args.push('--', ...spec.agentCommand)
  return args
}

async function resolveMountedResources(
  resources: readonly unknown[],
  cwd: string,
): Promise<MountedResource[]> {
  const mountedResources: MountedResource[] = []
  for (const [index, resource] of resources.entries()) {
    const parsed = parseLocalPathResource(resource, index)
    if (!isAbsolute(parsed.mount_path)) {
      throw new Error(
        `embedded-spec boot requires absolute mount paths; got '${parsed.mount_path}'`,
      )
    }

    const resolvedSourcePath = resolvePath(cwd, parsed.source_ref.path)
    let hostPath: string
    try {
      hostPath = await realpath(resolvedSourcePath)
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      throw new Error(
        `embedded-spec boot could not resolve local resource path '${parsed.source_ref.path}': ${message}`,
      )
    }

    mountedResources.push({
      host_path: hostPath,
      mount_path: parsed.mount_path,
      read_only: parsed.read_only ?? true,
    })
  }

  return mountedResources
}

function parseLocalPathResource(value: unknown, index: number): {
  readonly source_ref: {
    readonly kind: 'localPath'
    readonly host_id: string
    readonly path: string
  }
  readonly mount_path: string
  readonly read_only?: boolean
} {
  if (!value || typeof value !== 'object') {
    throw new Error(`embedded-spec boot received malformed resource[${index}]`)
  }
  const resource = value as JsonRecord
  const sourceRef = resource.source_ref
  if (!sourceRef || typeof sourceRef !== 'object') {
    throw new Error(`embedded-spec boot received malformed resource[${index}].source_ref`)
  }
  const parsedSourceRef = sourceRef as JsonRecord
  const kind = parsedSourceRef.kind
  if (kind !== 'localPath') {
    throw new Error(
      `embedded-spec boot only supports localPath resource mounts today; got '${String(kind)}'`,
    )
  }

  const hostId = parsedSourceRef.host_id
  if (typeof hostId !== 'string' || hostId.length === 0) {
    throw new Error(`embedded-spec boot requires resource[${index}].source_ref.host_id`)
  }
  if (hostId !== 'local') {
    throw new Error(
      `embedded-spec boot only supports host_id='local' resource mounts; got '${hostId}'`,
    )
  }

  const path = parsedSourceRef.path
  if (typeof path !== 'string' || path.length === 0) {
    throw new Error(`embedded-spec boot requires resource[${index}].source_ref.path`)
  }

  const mountPath = resource.mount_path
  if (typeof mountPath !== 'string' || mountPath.length === 0) {
    throw new Error(`embedded-spec boot requires resource[${index}].mount_path`)
  }

  const readOnly = resource.read_only
  if (readOnly !== undefined && typeof readOnly !== 'boolean') {
    throw new Error(`embedded-spec boot requires resource[${index}].read_only to be boolean`)
  }

  return {
    source_ref: {
      kind: 'localPath',
      host_id: hostId,
      path,
    },
    mount_path: mountPath,
    ...(typeof readOnly === 'boolean' ? { read_only: readOnly } : {}),
  }
}

function hasTopologyComponents(topology: unknown): boolean {
  if (!topology || typeof topology !== 'object') {
    return false
  }
  const components = (topology as { readonly components?: unknown }).components
  return Array.isArray(components) && components.length > 0
}
