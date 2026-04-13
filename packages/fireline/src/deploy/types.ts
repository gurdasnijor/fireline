export type BuildTarget = 'cloudflare' | 'docker' | 'docker-compose' | 'fly' | 'k8s'
export type DeployTarget = 'cloudflare-containers' | 'docker-compose' | 'fly' | 'k8s'

export interface SerializedHarnessSpec extends Record<string, unknown> {
  readonly name: string
  readonly sandbox: Record<string, unknown>
}

export interface DockerBuildPlan {
  readonly command: 'docker'
  readonly args: readonly string[]
  readonly buildArg: string
  readonly buildContext: string
  readonly dockerfile: string
  readonly imageTag: string
}

export interface TargetScaffoldFile {
  readonly fileName: string
  readonly filePath: string
  readonly contents: string
}

export interface TargetScaffoldPlan {
  readonly target: BuildTarget
  readonly fileName: string
  readonly filePath: string
  readonly contents: string
  readonly files: readonly TargetScaffoldFile[]
}

export interface DeployExecutionStep {
  readonly label: string
  readonly command: string
  readonly args: readonly string[]
  readonly cwd: string
  readonly installHint: string
}

export interface DeployExecutionPlan {
  readonly target: DeployTarget
  readonly command: string
  readonly args: readonly string[]
  readonly cwd: string
  readonly installHint: string
  readonly deployImageRef: string
  readonly steps: readonly DeployExecutionStep[]
}
