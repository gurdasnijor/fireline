import type { BuildTarget, DeployTarget } from './types.js'

const QUICKSTART_DOCKERFILE = 'docker/fireline-host-quickstart.Dockerfile'
const DEFAULT_DEPLOY_IMAGE_ENV = 'FIRELINE_DEPLOY_IMAGE'
const DEFAULT_DEPLOY_REGISTRY_ENV = 'FIRELINE_DEPLOY_REGISTRY'

export function hostedDockerfileForTarget(target: BuildTarget | null): string {
  void target
  return QUICKSTART_DOCKERFILE
}

export function resolveDeployImageRef(options: {
  readonly target: DeployTarget | BuildTarget
  readonly appName: string
  readonly imageTag: string
  readonly environment?: NodeJS.ProcessEnv
}): string {
  const environment = options.environment ?? process.env
  const explicit = environment[DEFAULT_DEPLOY_IMAGE_ENV]?.trim()
  if (explicit) {
    return explicit
  }

  if (options.target === 'fly') {
    return `registry.fly.io/${options.appName}:latest`
  }

  const registry = normalizeRegistryPrefix(environment[DEFAULT_DEPLOY_REGISTRY_ENV])
  if (registry) {
    return `${registry}/fireline-${options.appName}:latest`
  }

  return options.imageTag
}

export function requiresPublishedImage(target: DeployTarget): boolean {
  return target === 'fly' || target === 'k8s'
}

export function deployNeedsImagePush(options: {
  readonly target: DeployTarget
  readonly imageTag: string
  readonly deployImageRef: string
}): boolean {
  if (options.target === 'cloudflare-containers') {
    return false
  }
  return options.imageTag !== options.deployImageRef
}

export function registryConfigurationHint(target: DeployTarget): string | null {
  switch (target) {
    case 'fly':
      return null
    case 'docker-compose':
      return null
    case 'cloudflare-containers':
      return null
    case 'k8s':
      return `Set ${DEFAULT_DEPLOY_IMAGE_ENV}=registry.example.com/team/fireline-app:latest or ${DEFAULT_DEPLOY_REGISTRY_ENV}=registry.example.com/team before deploying to k8s.`
  }
}

function normalizeRegistryPrefix(value: string | undefined): string | null {
  const trimmed = value?.trim()
  if (!trimmed) {
    return null
  }
  return trimmed.replace(/\/+$/g, '')
}
