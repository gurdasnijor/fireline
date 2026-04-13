import { existsSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { resolve as resolvePath } from 'node:path'

export type DeployTarget = 'cloudflare-containers' | 'docker-compose' | 'fly' | 'k8s'

export interface HostedResourceNaming {
  readonly appName?: string
  readonly imageTag?: string
  readonly namespace?: string
}

export interface HostedTargetConfig {
  readonly provider: DeployTarget
  readonly region?: string
  readonly resourceNaming?: HostedResourceNaming
  readonly authRef?: string
}

export interface HostedConfig {
  readonly defaultTarget: string | null
  readonly targets: Readonly<Record<string, HostedTargetConfig>>
}

export interface HostedDeployResolution {
  readonly configPath: string
  readonly targetName: string | null
  readonly target: HostedTargetConfig | null
  readonly deployTarget: DeployTarget
  readonly token: string | null
  readonly tokenSource: string | null
  readonly tokenSinkEnvVar: string
}

const INLINE_CONFIG_PATH = '<inline>'

export function defaultHostedConfigPath(): string {
  return resolvePath(homedir(), '.fireline', 'hosted.json')
}

export async function loadHostedConfig(
  configPath: string = defaultHostedConfigPath(),
): Promise<HostedConfig | null> {
  if (!existsSync(configPath)) {
    return null
  }
  return parseHostedConfig(await readFile(configPath, 'utf8'), configPath)
}

export function parseHostedConfig(
  rawText: string,
  configPath: string = INLINE_CONFIG_PATH,
): HostedConfig {
  let parsed: unknown
  try {
    parsed = JSON.parse(rawText)
  } catch (error) {
    throw new Error(
      `invalid hosted config JSON at ${configPath}: ${(error as Error).message}`,
    )
  }

  const root = expectRecord(parsed, configPath)
  const defaultTarget = readOptionalString(root.defaultTarget ?? root['default-target'], 'defaultTarget', configPath)
  const rawTargets = root.targets
  if (!rawTargets || typeof rawTargets !== 'object' || Array.isArray(rawTargets)) {
    throw new Error(`hosted config at ${configPath} requires an object 'targets' map`)
  }

  const targets: Record<string, HostedTargetConfig> = {}
  for (const [name, value] of Object.entries(rawTargets)) {
    targets[name] = parseHostedTarget(name, value, configPath)
  }

  return {
    defaultTarget: defaultTarget ?? null,
    targets,
  }
}

export function parseDeployTarget(
  value: string | undefined,
  flag: string = '--to',
): DeployTarget {
  const normalized = required(value, flag).toLowerCase()
  switch (normalized) {
    case 'cloudflare-containers':
    case 'cloudflare':
      return 'cloudflare-containers'
    case 'docker-compose':
    case 'compose':
      return 'docker-compose'
    case 'fly':
    case 'flyio':
      return 'fly'
    case 'k8s':
    case 'kubernetes':
      return 'k8s'
    default:
      throw new Error(
        `unsupported deploy target: ${normalized} (expected fly, cloudflare-containers, docker-compose, or k8s)`,
      )
  }
}

export function providerTokenEnvVar(provider: DeployTarget): string | null {
  switch (provider) {
    case 'fly':
      return 'FLY_API_TOKEN'
    case 'cloudflare-containers':
      return 'CLOUDFLARE_API_TOKEN'
    case 'docker-compose':
    case 'k8s':
      return null
  }
}

export function targetTokenEnvVar(targetName: string): string {
  return `FIRELINE_${targetName
    .replace(/[^A-Za-z0-9]+/g, '_')
    .replace(/^_+|_+$/g, '')
    .toUpperCase()}_TOKEN`
}

export function resolveHostedDeploy(options: {
  readonly config: HostedConfig | null
  readonly configPath?: string
  readonly targetName: string | null
  readonly deployTarget: DeployTarget | null
  readonly token: string | null
  readonly env: NodeJS.ProcessEnv
}): HostedDeployResolution {
  const configPath = options.configPath ?? defaultHostedConfigPath()
  const targetName = options.targetName ?? options.config?.defaultTarget ?? null
  const target = targetName ? resolveHostedTarget(options.config, targetName, configPath) : null

  if (target && options.deployTarget && target.provider !== options.deployTarget) {
    throw new Error(
      `--target ${targetName} resolves to provider '${target.provider}', which conflicts with --to ${options.deployTarget}`,
    )
  }

  const deployTarget = options.deployTarget ?? target?.provider ?? null
  if (!deployTarget) {
    throw new Error(
      `deploy requires --to <platform> or a configured target in ${configPath}`,
    )
  }

  const sinkEnvVar =
    target?.authRef ??
    providerTokenEnvVar(deployTarget) ??
    (targetName ? targetTokenEnvVar(targetName) : null) ??
    'FIRELINE_TOKEN'

  if (options.token) {
    return {
      configPath,
      targetName,
      target,
      deployTarget,
      token: options.token,
      tokenSource: '--token',
      tokenSinkEnvVar: sinkEnvVar,
    }
  }

  const tokenSources = [
    target?.authRef ? { envVar: target.authRef, source: `env:${target.authRef}` } : null,
    targetName
      ? {
          envVar: targetTokenEnvVar(targetName),
          source: `env:${targetTokenEnvVar(targetName)}`,
        }
      : null,
    providerTokenEnvVar(deployTarget)
      ? {
          envVar: providerTokenEnvVar(deployTarget)!,
          source: `env:${providerTokenEnvVar(deployTarget)!}`,
        }
      : null,
    { envVar: 'FIRELINE_TOKEN', source: 'env:FIRELINE_TOKEN' },
  ].filter((entry): entry is { envVar: string; source: string } => Boolean(entry))

  const seen = new Set<string>()
  for (const candidate of tokenSources) {
    if (seen.has(candidate.envVar)) {
      continue
    }
    seen.add(candidate.envVar)
    const value = options.env[candidate.envVar]
    if (value && value.trim()) {
      return {
        configPath,
        targetName,
        target,
        deployTarget,
        token: value,
        tokenSource: candidate.source,
        tokenSinkEnvVar: sinkEnvVar,
      }
    }
  }

  return {
    configPath,
    targetName,
    target,
    deployTarget,
    token: null,
    tokenSource: null,
    tokenSinkEnvVar: sinkEnvVar,
  }
}

function parseHostedTarget(
  name: string,
  value: unknown,
  configPath: string,
): HostedTargetConfig {
  const target = expectRecord(value, `${configPath} targets.${name}`)
  const resourceNamingValue = target.resourceNaming ?? target['resource-naming']

  return {
    provider: parseDeployTarget(
      readRequiredString(target.provider, `targets.${name}.provider`, configPath),
      `targets.${name}.provider`,
    ),
    region: readOptionalString(target.region, `targets.${name}.region`, configPath),
    resourceNaming: resourceNamingValue
      ? parseResourceNaming(resourceNamingValue, `targets.${name}.resourceNaming`, configPath)
      : undefined,
    authRef: readOptionalString(
      target.authRef ?? target['auth-ref'],
      `targets.${name}.authRef`,
      configPath,
    ),
  }
}

function parseResourceNaming(
  value: unknown,
  field: string,
  configPath: string,
): HostedResourceNaming {
  const record = expectRecord(value, `${configPath} ${field}`)
  const appName = readOptionalString(record.appName ?? record['app-name'], `${field}.appName`, configPath)
  const imageTag = readOptionalString(record.imageTag ?? record['image-tag'], `${field}.imageTag`, configPath)
  const namespace = readOptionalString(record.namespace, `${field}.namespace`, configPath)
  return {
    ...(appName ? { appName } : {}),
    ...(imageTag ? { imageTag } : {}),
    ...(namespace ? { namespace } : {}),
  }
}

function resolveHostedTarget(
  config: HostedConfig | null,
  targetName: string,
  configPath: string,
): HostedTargetConfig {
  if (!config) {
    throw new Error(
      `--target ${targetName} requires hosted config at ${configPath}`,
    )
  }
  const target = config.targets[targetName]
  if (!target) {
    throw new Error(`hosted config at ${configPath} does not define target '${targetName}'`)
  }
  return target
}

function expectRecord(value: unknown, location: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${location} must be an object`)
  }
  return value as Record<string, unknown>
}

function readRequiredString(value: unknown, field: string, configPath: string): string {
  const parsed = readOptionalString(value, field, configPath)
  if (!parsed) {
    throw new Error(`${configPath} ${field} must be a non-empty string`)
  }
  return parsed
}

function readOptionalString(
  value: unknown,
  field: string,
  configPath: string,
): string | undefined {
  if (value === undefined || value === null) {
    return undefined
  }
  if (typeof value !== 'string' || !value.trim()) {
    throw new Error(`${configPath} ${field} must be a non-empty string`)
  }
  return value
}

function required(value: string | undefined, flag: string): string {
  if (value === undefined) {
    throw new Error(`${flag} requires an argument`)
  }
  return value
}
