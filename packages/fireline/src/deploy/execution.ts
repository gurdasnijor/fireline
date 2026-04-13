import {
  deployNeedsImagePush,
  registryConfigurationHint,
  requiresPublishedImage,
  resolveDeployImageRef,
} from './images.js'
import type {
  DeployExecutionPlan,
  DeployExecutionStep,
  DeployTarget,
} from './types.js'

const INSTALL_HINTS = {
  docker: 'Install Docker Desktop (`brew install --cask docker`) or Docker Engine with the Compose plugin: https://docs.docker.com/compose/install/',
  flyctl: 'Install flyctl (`brew install flyctl`): https://fly.io/docs/flyctl/install/',
  kubectl: 'Install kubectl (`brew install kubectl`): https://kubernetes.io/docs/tasks/tools/',
  wrangler: 'Install Wrangler (`pnpm add -D wrangler` or `npm install -D wrangler`): https://developers.cloudflare.com/workers/wrangler/install-and-update/',
} as const

export function createDeployExecutionPlan(options: {
  readonly target: DeployTarget
  readonly cwd: string
  readonly imageTag: string
  readonly scaffoldPath: string | null
  readonly passthroughArgs: readonly string[]
  readonly appName?: string
  readonly environment?: NodeJS.ProcessEnv
}): DeployExecutionPlan {
  const appName = options.appName ?? inferAppNameFromImageTag(options.imageTag)
  const environment = options.environment ?? process.env
  const deployImageRef = resolveDeployImageRef({
    target: options.target,
    appName,
    imageTag: options.imageTag,
    environment,
  })
  const scaffoldPath = requireScaffoldPath(options.target, options.scaffoldPath)

  if (requiresPublishedImage(options.target) && deployImageRef === options.imageTag) {
    const hint = registryConfigurationHint(options.target)
    throw new Error(
      [
        `deploy target '${options.target}' requires a pullable image reference instead of the local build tag '${options.imageTag}'.`,
        ...(hint ? [hint] : []),
      ].join('\n'),
    )
  }

  const steps: DeployExecutionStep[] = []
  if (options.target === 'fly') {
    steps.push(step('Authenticate docker against Fly registry', 'flyctl', ['auth', 'docker'], options.cwd))
  }

  if (deployNeedsImagePush({
    target: options.target,
    imageTag: options.imageTag,
    deployImageRef,
  })) {
    steps.push(step(`Tag ${options.imageTag} for deployment`, 'docker', ['tag', options.imageTag, deployImageRef], options.cwd))
    steps.push(step(`Push ${deployImageRef}`, 'docker', ['push', deployImageRef], options.cwd))
  }

  switch (options.target) {
    case 'cloudflare-containers':
      steps.push(step('Deploy Cloudflare Containers manifest', 'wrangler', ['deploy', '--config', scaffoldPath, ...options.passthroughArgs], options.cwd))
      break
    case 'docker-compose':
      steps.push(step('Bring up docker compose services', 'docker', ['compose', '-f', scaffoldPath, 'up', '-d', ...options.passthroughArgs], options.cwd))
      break
    case 'fly':
      steps.push(step('Deploy the image on Fly', 'flyctl', ['deploy', '--config', scaffoldPath, '--image', deployImageRef, ...options.passthroughArgs], options.cwd))
      break
    case 'k8s':
      steps.push(step('Apply Kubernetes manifests', 'kubectl', ['apply', '-f', scaffoldPath, ...options.passthroughArgs], options.cwd))
      break
  }

  const finalStep = steps[steps.length - 1]
  return {
    target: options.target,
    command: finalStep.command,
    args: finalStep.args,
    cwd: finalStep.cwd,
    installHint: finalStep.installHint,
    deployImageRef,
    steps,
  }
}

export function decorateMissingDeployToolError(step: DeployExecutionStep, error: unknown): Error {
  const message = (error as Error)?.message ?? String(error)
  return new Error(`${message}\n${step.command} is required for "${step.label}".\n${step.installHint}`)
}

function requireScaffoldPath(target: DeployTarget, scaffoldPath: string | null): string {
  if (!scaffoldPath) {
    throw new Error(`deploy target '${target}' requires a generated manifest`)
  }
  return scaffoldPath
}

function inferAppNameFromImageTag(imageTag: string): string {
  const candidate = imageTag
    .replace(/^.*\//, '')
    .replace(/:.*$/, '')
    .replace(/^fireline-/, '')
    .trim()
  return candidate || 'default'
}

function step(label: string, command: keyof typeof INSTALL_HINTS, args: readonly string[], cwd: string): DeployExecutionStep {
  return {
    label,
    command,
    args,
    cwd,
    installHint: INSTALL_HINTS[command],
  }
}
