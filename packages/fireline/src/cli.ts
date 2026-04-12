import { ChildProcess, spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { basename, dirname, parse as parsePath, resolve as resolvePath } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { tsImport } from 'tsx/esm/api'
import { resolveBinary } from './resolve-binary.js'

export type BuildTarget = 'cloudflare' | 'docker' | 'docker-compose' | 'fly' | 'k8s'
export type DeployTarget = 'cloudflare-containers' | 'docker-compose' | 'fly' | 'k8s'

export interface ParsedArgs {
  readonly command: 'run' | 'build' | 'deploy' | 'agents' | 'help'
  readonly helpFor: 'general' | 'run' | 'build' | 'deploy' | 'agents'
  readonly file: string | null
  readonly passthroughArgs: readonly string[]
  readonly port: number
  readonly streamsPort: number
  readonly stateStream: string | null
  readonly name: string | null
  readonly repl: boolean
  readonly providerOverride: string | null
  readonly target: BuildTarget | null
  readonly to: DeployTarget | null
}

export interface DockerBuildPlan {
  readonly command: 'docker'
  readonly args: readonly string[]
  readonly buildArg: string
  readonly buildContext: string
  readonly dockerfile: string
  readonly imageTag: string
}

export interface TargetScaffoldPlan {
  readonly target: BuildTarget
  readonly fileName: string
  readonly filePath: string
  readonly contents: string
}

export interface DeployExecutionPlan {
  readonly target: DeployTarget
  readonly command: string
  readonly args: readonly string[]
  readonly cwd: string
  readonly installHint: string
}

interface HostedBuildResult {
  readonly exitCode: number
  readonly imageTag: string
  readonly scaffoldPlan: TargetScaffoldPlan | null
}

export interface CliRuntime {
  readonly cwd: () => string
  readonly loadSpec: (specPath: string) => Promise<LoadedSpec>
  readonly runChild: (
    command: string,
    args: readonly string[],
    options?: { readonly cwd?: string },
  ) => Promise<number>
  readonly writeTargetScaffold: (plan: TargetScaffoldPlan) => Promise<void>
}

const defaultCliRuntime: CliRuntime = {
  cwd: () => process.cwd(),
  loadSpec,
  runChild,
  writeTargetScaffold,
}

const GENERAL_HELP = `
fireline — run specs locally, build hosted images, deploy them, or install ACP agents

Usage:
  fireline run <file.ts> [flags]     Boot conductor + streams, provision agent locally
  fireline <file.ts> [flags]         Shorthand for run
  fireline build <file.ts> [flags]   Build hosted Fireline OCI image
  fireline deploy <file.ts> --to <platform> [-- <native-flags...>]
  fireline agents <command> [args]   Install ACP agents from the public registry
  fireline --help                    Show this help

Run flags:
  --port <n>           ACP control-plane port (default: 4440)
  --streams-port <n>   Durable-streams port   (default: 7474)
  --state-stream <s>   Explicit durable state stream name (enables resume)
  --name <s>           Logical agent name     (default: from spec or 'default')
  --provider <p>       Override sandbox.provider from spec
  --repl               Print ACP URL and wait (TODO: interactive REPL)

Build flags:
  --target <platform>  Scaffold target config: cloudflare | docker | docker-compose | fly | k8s
  --state-stream <s>   Override durable state stream name baked into the spec
  --name <s>           Override deployment name baked into the spec
  --provider <p>       Override sandbox.provider baked into the spec

Deploy flags:
  --to <platform>      Native deploy target: fly | cloudflare-containers | docker-compose | k8s
  --state-stream <s>   Override durable state stream name baked into the spec
  --name <s>           Override deployment name baked into the spec
  --provider <p>       Override sandbox.provider baked into the spec
  --                   Pass remaining args through to the native target CLI

Env:
  FIRELINE_BIN          Override path to fireline binary
  FIRELINE_STREAMS_BIN  Override path to fireline-streams binary
  FIRELINE_AGENTS_BIN   Override path to fireline-agents binary

Example:
  fireline run packages/fireline/test-fixtures/minimal-spec.ts
  fireline build agent.ts --target fly
  fireline deploy agent.ts --to fly -- --remote-only
  fireline agents add pi-acp
`.trim()

const RUN_HELP = `
fireline run — boot Fireline locally and provision a spec

Usage:
  fireline run <file.ts> [flags]
  fireline <file.ts> [flags]

Flags:
  --port <n>           ACP control-plane port (default: 4440)
  --streams-port <n>   Durable-streams port   (default: 7474)
  --state-stream <s>   Explicit durable state stream name (enables resume)
  --name <s>           Logical agent name     (default: from spec or 'default')
  --provider <p>       Override sandbox.provider from spec
  --repl               Print ACP URL and wait (TODO: interactive REPL)
  --help               Show this help
`.trim()

const BUILD_HELP = `
fireline build — build a hosted Fireline OCI image from a spec

Usage:
  fireline build <file.ts> [flags]

Flags:
  --target <platform>  Scaffold target config: cloudflare | docker | docker-compose | fly | k8s
  --state-stream <s>   Override durable state stream name baked into the spec
  --name <s>           Override deployment name baked into the spec
  --provider <p>       Override sandbox.provider baked into the spec
  --help               Show this help
`.trim()

const DEPLOY_HELP = `
fireline deploy — build a hosted image and hand off to a native platform CLI

Usage:
  fireline deploy <file.ts> --to <platform> [flags] [-- <native-flags...>]

Flags:
  --to <platform>      Native deploy target: fly | cloudflare-containers | docker-compose | k8s
  --state-stream <s>   Override durable state stream name baked into the spec
  --name <s>           Override deployment name baked into the spec
  --provider <p>       Override sandbox.provider baked into the spec
  --help               Show this help

Native CLIs:
  fly                   flyctl deploy
  cloudflare-containers wrangler deploy
  docker-compose        docker compose up -d
  k8s                   kubectl apply -f <generated>

Example:
  fireline deploy agent.ts --to fly -- --remote-only
`.trim()

const AGENTS_HELP = `
fireline agents — install ACP agents from the public registry

Usage:
  fireline agents add <id>
  fireline agents --help

Commands:
  add <id>             Install an ACP agent by registry id

Env:
  FIRELINE_AGENTS_BIN  Override path to fireline-agents binary

Example:
  fireline agents add pi-acp
`.trim()

export async function main(argv: readonly string[]): Promise<void> {
  let exitCode = 0
  try {
    const args = parseArgs(argv)
    if (args.command === 'help' || (args.command !== 'agents' && !args.file)) {
      console.log(helpText(args.helpFor))
      return
    }
    exitCode = args.command === 'build'
      ? await build(args)
      : args.command === 'deploy'
        ? await deploy(args)
      : args.command === 'agents'
        ? await runAgents(args)
        : await run(args)
  } catch (error) {
    console.error(`fireline: ${(error as Error).message}`)
    exitCode = 1
  }
  process.exit(exitCode)
}

export function parseArgs(argv: readonly string[]): ParsedArgs {
  const out = {
    command: 'run' as 'run' | 'build' | 'deploy' | 'agents' | 'help',
    helpFor: 'run' as 'general' | 'run' | 'build' | 'deploy' | 'agents',
    file: null as string | null,
    passthroughArgs: [] as string[],
    port: 4440,
    streamsPort: 7474,
    stateStream: null as string | null,
    name: null as string | null,
    repl: false,
    providerOverride: null as string | null,
    target: null as BuildTarget | null,
    to: null as DeployTarget | null,
  }
  const seen = {
    port: false,
    streamsPort: false,
    repl: false,
    target: false,
    to: false,
  }
  let i = 0
  if (argv[0] === 'run' || argv[0] === 'build' || argv[0] === 'deploy' || argv[0] === 'agents') {
    out.command = argv[0]
    out.helpFor = argv[0]
    i++
  }
  if (argv[0] === '--help' || argv[0] === '-h') {
    return { ...out, command: 'help', helpFor: 'general' }
  }

  for (; i < argv.length; i++) {
    const arg = argv[i]
    if (out.command === 'agents') {
      if (arg === '--help' || arg === '-h') {
        return { ...out, command: 'help', helpFor: 'agents' }
      }
      out.passthroughArgs = [...out.passthroughArgs, arg]
      continue
    }
    if (out.command === 'deploy' && arg === '--') {
      out.passthroughArgs = argv.slice(i + 1)
      break
    }
    switch (arg) {
      case '--help':
      case '-h':
        return { ...out, command: 'help' }
      case '--port':
        seen.port = true
        out.port = parseIntArg(argv[++i], '--port')
        break
      case '--streams-port':
        seen.streamsPort = true
        out.streamsPort = parseIntArg(argv[++i], '--streams-port')
        break
      case '--state-stream':
        out.stateStream = required(argv[++i], '--state-stream')
        break
      case '--name':
        out.name = required(argv[++i], '--name')
        break
      case '--provider':
        out.providerOverride = required(argv[++i], '--provider')
        break
      case '--target':
        seen.target = true
        out.target = parseBuildTarget(argv[++i])
        break
      case '--to':
        seen.to = true
        out.to = parseDeployTarget(argv[++i])
        break
      case '--repl':
        seen.repl = true
        out.repl = true
        break
      default:
        if (arg?.startsWith('--')) throw new Error(`unknown flag: ${arg}`)
        if (out.file) throw new Error(`unexpected argument: ${arg}`)
        out.file = arg
    }
  }

  if (out.command === 'run' && seen.target) {
    throw new Error('--target is only valid with build')
  }
  if (out.command === 'run' && seen.to) {
    throw new Error('--to is only valid with deploy')
  }
  if (out.command === 'build' && seen.port) {
    throw new Error('--port is only valid with run')
  }
  if (out.command === 'build' && seen.streamsPort) {
    throw new Error('--streams-port is only valid with run')
  }
  if (out.command === 'build' && seen.repl) {
    throw new Error('--repl is only valid with run')
  }
  if (out.command === 'build' && seen.to) {
    throw new Error('--to is only valid with deploy')
  }
  if (out.command === 'deploy' && seen.target) {
    throw new Error('--target is only valid with build')
  }
  if (out.command === 'deploy' && seen.port) {
    throw new Error('--port is only valid with run')
  }
  if (out.command === 'deploy' && seen.streamsPort) {
    throw new Error('--streams-port is only valid with run')
  }
  if (out.command === 'deploy' && seen.repl) {
    throw new Error('--repl is only valid with run')
  }
  if (out.command === 'deploy' && !seen.to) {
    throw new Error('deploy requires --to <platform>')
  }

  return out
}

function helpText(topic: ParsedArgs['helpFor']): string {
  switch (topic) {
    case 'run':
      return RUN_HELP
    case 'build':
      return BUILD_HELP
    case 'deploy':
      return DEPLOY_HELP
    case 'agents':
      return AGENTS_HELP
    case 'general':
      return GENERAL_HELP
  }
}

async function runAgents(args: ParsedArgs): Promise<number> {
  const agentsBin = resolveBinary({ name: 'fireline-agents', envVar: 'FIRELINE_AGENTS_BIN' })
  return await runChild(agentsBin, args.passthroughArgs)
}

function parseIntArg(value: string | undefined, flag: string): number {
  const n = Number.parseInt(required(value, flag), 10)
  if (!Number.isFinite(n) || n <= 0) throw new Error(`${flag} must be a positive integer`)
  return n
}

function required(value: string | undefined, flag: string): string {
  if (value === undefined) throw new Error(`${flag} requires an argument`)
  return value
}

function parseBuildTarget(value: string | undefined): BuildTarget {
  const normalized = required(value, '--target').toLowerCase()
  switch (normalized) {
    case 'cloudflare':
    case 'cf':
      return 'cloudflare'
    case 'docker':
      return 'docker'
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
      throw new Error(`unsupported build target: ${normalized} (expected cloudflare, docker, docker-compose, fly, or k8s)`)
  }
}

function parseDeployTarget(value: string | undefined): DeployTarget {
  const normalized = required(value, '--to').toLowerCase()
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
      throw new Error(`unsupported deploy target: ${normalized} (expected fly, cloudflare-containers, docker-compose, or k8s)`)
  }
}

async function run(args: ParsedArgs): Promise<number> {
  const specPath = resolvePath(process.cwd(), args.file!)
  const spec = await loadSpec(specPath)

  const streamsBin = resolveBinary({ name: 'fireline-streams', envVar: 'FIRELINE_STREAMS_BIN' })
  const firelineBin = resolveBinary({ name: 'fireline', envVar: 'FIRELINE_BIN' })

  const teardown: Array<() => Promise<void> | void> = []
  let shutdownSignal: number | null = null
  const waitForShutdown = new Promise<number>((resolveWait) => {
    const onSignal = (code: number) => () => {
      if (shutdownSignal !== null) return
      shutdownSignal = code
      resolveWait(code)
    }
    process.once('SIGINT', onSignal(130))
    process.once('SIGTERM', onSignal(143))
  })

  async function runTeardown(): Promise<void> {
    for (const fn of teardown.reverse()) {
      try { await fn() } catch (error) {
        console.error(`fireline: teardown error: ${(error as Error).message}`)
      }
    }
  }

  try {
    const streamsProc = spawn(streamsBin, [], {
      stdio: ['ignore', 'inherit', 'inherit'],
      env: { ...process.env, PORT: String(args.streamsPort) },
    })
    teardown.push(() => stopChild(streamsProc))
    await waitForHttp(`http://127.0.0.1:${args.streamsPort}/healthz`, 10_000, 'fireline-streams')

    const controlPlaneArgs = [
      '--control-plane',
      '--port', String(args.port),
      '--durable-streams-url', `http://127.0.0.1:${args.streamsPort}/v1/stream`,
    ]
    const firelineProc = spawn(firelineBin, controlPlaneArgs, {
      stdio: ['ignore', 'inherit', 'inherit'],
      env: { ...process.env },
    })
    teardown.push(() => stopChild(firelineProc))
    await waitForHttp(`http://127.0.0.1:${args.port}/healthz`, 15_000, 'fireline')

    const startOptions: Record<string, unknown> = {
      serverUrl: `http://127.0.0.1:${args.port}`,
    }
    if (args.stateStream) startOptions.stateStream = args.stateStream
    if (args.name) startOptions.name = args.name

    const effectiveSpec = args.providerOverride
      ? { ...spec, sandbox: { ...spec.sandbox, provider: args.providerOverride } }
      : spec

    const agentHandle = await effectiveSpec.start(startOptions)
    teardown.push(() => destroySandbox(agentHandle.id, args.port))

    printReady(agentHandle, args)
    if (args.repl) {
      console.log('\nREPL mode coming soon. Connect any ACP client to the URL above.')
    }

    const signalCode = await waitForShutdown
    await runTeardown()
    return signalCode
  } catch (error) {
    await runTeardown()
    throw error
  }
}

export async function build(
  args: ParsedArgs,
  runtime: CliRuntime = defaultCliRuntime,
): Promise<number> {
  return (await executeHostedBuild(args, runtime)).exitCode
}

export async function deploy(
  args: ParsedArgs,
  runtime: CliRuntime = defaultCliRuntime,
): Promise<number> {
  const target = args.to
  if (!target) {
    throw new Error('deploy requires --to <platform>')
  }

  const scaffoldCwd = await mkdtemp(resolvePath(tmpdir(), 'fireline-deploy-'))
  try {
    const buildResult = await executeHostedBuild(args, runtime, {
      scaffoldTarget: deployScaffoldTarget(target),
      scaffoldCwd,
      printResult: false,
    })
    if (buildResult.exitCode !== 0) return buildResult.exitCode

    const nativePlan = createDeployExecutionPlan({
      target,
      cwd: runtime.cwd(),
      imageTag: buildResult.imageTag,
      scaffoldPath: buildResult.scaffoldPlan?.filePath ?? null,
      passthroughArgs: args.passthroughArgs,
    })

    console.log(`fireline: deploying ${buildResult.imageTag} via ${nativePlan.command}`)
    try {
      const exitCode = await runtime.runChild(nativePlan.command, nativePlan.args, { cwd: nativePlan.cwd })
      if (exitCode === 0) {
        printDeployResult(buildResult.imageTag, target)
      }
      return exitCode
    } catch (error) {
      throw decorateMissingDeployToolError(nativePlan, error)
    }
  } finally {
    await rm(scaffoldCwd, { recursive: true, force: true })
  }
}

export interface LoadedSpec {
  readonly kind: 'harness'
  readonly name: string
  readonly stateStream?: string
  readonly sandbox: { readonly provider?: unknown }
  readonly middleware: unknown
  readonly agent: unknown
  readonly start: (options: Record<string, unknown>) => Promise<{ readonly id: string; readonly acp: { readonly url: string }; readonly state: { readonly url: string } }>
}

async function loadSpec(specPath: string): Promise<LoadedSpec> {
  const parentURL = pathToFileURL(`${dirname(specPath)}/`).href
  const mod = await tsImport(`./${basename(specPath)}`, parentURL)
  const candidate = (mod as { default?: unknown }).default
  if (!candidate || typeof candidate !== 'object') {
    throw new Error(
      `${specPath} must have a default export. Got: ${typeof candidate}\n` +
        `Hint: export default compose(sandbox(), middleware([...]), agent([...]))`,
    )
  }
  if (typeof (candidate as { start?: unknown }).start !== 'function') {
    throw new Error(
      `${specPath} default export is not a Harness (no .start() method).\n` +
        `Hint: wrap it with compose() from '@fireline/client'`,
    )
  }
  return candidate as LoadedSpec
}

async function waitForHttp(url: string, timeoutMs: number, label: string): Promise<void> {
  const deadline = Date.now() + timeoutMs
  let lastError: unknown = null
  while (Date.now() < deadline) {
    try {
      const res = await fetch(url, { signal: AbortSignal.timeout(500) })
      if (res.ok) return
      lastError = new Error(`${label} healthz returned ${res.status}`)
    } catch (error) {
      lastError = error
    }
    await sleep(100)
  }
  throw new Error(`${label} failed to become healthy at ${url}: ${(lastError as Error)?.message ?? 'timeout'}`)
}

async function destroySandbox(id: string, port: number): Promise<void> {
  try {
    await fetch(`http://127.0.0.1:${port}/v1/sandboxes/${encodeURIComponent(id)}`, {
      method: 'DELETE',
      signal: AbortSignal.timeout(3_000),
    })
  } catch {
    // sandbox may already be gone if control plane crashed; ignore
  }
}

async function stopChild(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.killed) return
  child.kill('SIGTERM')
  await new Promise<void>((resolveWait) => {
    const timeout = setTimeout(() => {
      if (child.exitCode === null) child.kill('SIGKILL')
    }, 3_000)
    child.once('exit', () => { clearTimeout(timeout); resolveWait() })
  })
}

async function runChild(
  command: string,
  args: readonly string[],
  options: { readonly cwd?: string } = {},
): Promise<number> {
  const child = spawn(command, [...args], {
    cwd: options.cwd,
    stdio: ['ignore', 'inherit', 'inherit'],
    env: { ...process.env },
  })
  return await new Promise<number>((resolveWait, reject) => {
    child.once('error', (error) => {
      reject(new Error(`failed to start ${command}: ${(error as Error).message}`))
    })
    child.once('exit', (code, signal) => {
      if (signal) {
        reject(new Error(`${command} exited from signal ${signal}`))
        return
      }
      resolveWait(code ?? 1)
    })
  })
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolveWait) => setTimeout(resolveWait, ms))
}

interface PrintedHandle {
  readonly id: string
  readonly acp: { readonly url: string }
  readonly state: { readonly url: string }
}

export interface SerializedHarnessSpec extends Record<string, unknown> {
  readonly name: string
  readonly sandbox: Record<string, unknown>
}

function materializeSpec(spec: LoadedSpec, args: ParsedArgs): SerializedHarnessSpec {
  const baseSpec = serializeHarnessSpec(spec)
  return {
    ...baseSpec,
    ...(args.name ? { name: args.name } : {}),
    ...(args.stateStream ? { stateStream: args.stateStream } : {}),
    ...(args.providerOverride
      ? {
          sandbox: {
            ...baseSpec.sandbox,
            provider: args.providerOverride,
          },
        }
      : {}),
  }
}

function serializeHarnessSpec(spec: LoadedSpec): SerializedHarnessSpec {
  return JSON.parse(JSON.stringify(spec)) as SerializedHarnessSpec
}

export function createDockerBuildPlan(options: {
  readonly buildContext: string
  readonly dockerfile: string
  readonly imageTag: string
  readonly spec: SerializedHarnessSpec
}): DockerBuildPlan {
  const buildArg = `FIRELINE_EMBEDDED_SPEC=${JSON.stringify(options.spec)}`
  return {
    command: 'docker',
    args: [
      'build',
      '--file', options.dockerfile,
      '--tag', options.imageTag,
      '--build-arg', buildArg,
      options.buildContext,
    ],
    buildArg,
    buildContext: options.buildContext,
    dockerfile: options.dockerfile,
    imageTag: options.imageTag,
  }
}

export function createTargetScaffoldPlan(options: {
  readonly target: BuildTarget
  readonly cwd: string
  readonly appName: string
  readonly imageTag: string
}): TargetScaffoldPlan {
  const fileName = scaffoldFileName(options.target)
  const filePath = resolvePath(options.cwd, fileName)
  if (existsSync(filePath)) {
    throw new Error(`refusing to overwrite existing scaffold file: ${filePath}`)
  }
  return {
    target: options.target,
    fileName,
    filePath,
    contents: renderTargetScaffold(options),
  }
}

export async function writeTargetScaffold(plan: TargetScaffoldPlan): Promise<void> {
  await writeFile(plan.filePath, plan.contents, { flag: 'wx' })
}

function scaffoldFileName(target: BuildTarget): string {
  switch (target) {
    case 'cloudflare':
      return 'wrangler.toml'
    case 'docker':
      return 'Dockerfile'
    case 'docker-compose':
      return 'docker-compose.yml'
    case 'fly':
      return 'fly.toml'
    case 'k8s':
      return 'k8s.yaml'
  }
}

function renderTargetScaffold(options: {
  readonly target: BuildTarget
  readonly appName: string
  readonly imageTag: string
}): string {
  switch (options.target) {
    case 'cloudflare':
      return [
        '# Scaffold only. Wire this image into your Cloudflare Containers config.',
        `name = "${options.appName}"`,
        `compatibility_date = "${new Date().toISOString().slice(0, 10)}"`,
        '',
        '[vars]',
        `FIRELINE_IMAGE = "${options.imageTag}"`,
        'FIRELINE_PORT = "4440"',
        '',
      ].join('\n')
    case 'docker':
      return [
        '# Scaffold only. This target reuses the image built by `fireline build`.',
        `FROM ${options.imageTag}`,
        '',
        'EXPOSE 4440',
        '',
      ].join('\n')
    case 'docker-compose':
      return [
        'services:',
        '  fireline:',
        `    image: ${options.imageTag}`,
        '    ports:',
        '      - "4440:4440"',
        '',
      ].join('\n')
    case 'fly':
      return [
        `app = "${options.appName}"`,
        'primary_region = "sea"',
        '',
        '[build]',
        `  image = "${options.imageTag}"`,
        '',
        '[http_service]',
        '  internal_port = 4440',
        '  force_https = true',
        '  auto_start_machines = true',
        '  auto_stop_machines = "off"',
        '',
        '  [[http_service.checks]]',
        '    grace_period = "20s"',
        '    interval = "15s"',
        '    method = "GET"',
        '    path = "/healthz"',
        '    timeout = "5s"',
        '',
      ].join('\n')
    case 'k8s':
      return [
        'apiVersion: apps/v1',
        'kind: Deployment',
        'metadata:',
        `  name: ${options.appName}`,
        'spec:',
        '  replicas: 1',
        '  selector:',
        '    matchLabels:',
        `      app: ${options.appName}`,
        '  template:',
        '    metadata:',
        '      labels:',
        `        app: ${options.appName}`,
        '    spec:',
        '      containers:',
        `        - name: ${options.appName}`,
        `          image: ${options.imageTag}`,
        '          ports:',
        '            - containerPort: 4440',
        '          readinessProbe:',
        '            httpGet:',
        '              path: /healthz',
        '              port: 4440',
        '          livenessProbe:',
        '            httpGet:',
        '              path: /healthz',
        '              port: 4440',
        '---',
        'apiVersion: v1',
        'kind: Service',
        'metadata:',
        `  name: ${options.appName}`,
        'spec:',
        '  selector:',
        `    app: ${options.appName}`,
        '  ports:',
        '    - port: 80',
        '      targetPort: 4440',
        '',
      ].join('\n')
  }
}

function findWorkspacePath(relativePath: string): string {
  let dir = dirname(fileURLToPath(import.meta.url))
  for (let i = 0; i < 10; i++) {
    const candidate = resolvePath(dir, relativePath)
    if (existsSync(candidate)) return candidate
    const parent = dirname(dir)
    if (parent === dir) break
    dir = parent
  }
  throw new Error(`could not locate ${relativePath} from ${import.meta.url}`)
}

function defaultAppName(specPath: string, name: string): string {
  if (name && name !== 'default') return slugify(name)
  return slugify(parsePath(specPath).name || 'default')
}

function defaultImageTag(appName: string): string {
  return `fireline-${appName}:latest`
}

function deployScaffoldTarget(target: DeployTarget): BuildTarget {
  switch (target) {
    case 'cloudflare-containers':
      return 'cloudflare'
    case 'docker-compose':
      return 'docker-compose'
    case 'fly':
      return 'fly'
    case 'k8s':
      return 'k8s'
  }
}

function slugify(value: string): string {
  const slug = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  if (slug) return slug
  return slugify(parsePath(value).name || 'default')
}

function validatePrintedHandle(handle: PrintedHandle, label: string): void {
  if (!handle?.id || !handle.acp?.url || !handle.state?.url) {
    throw new Error(`${label} response missing id/acp/state endpoints`)
  }
}

function printReady(
  handle: PrintedHandle,
  args: ParsedArgs,
): void {
  validatePrintedHandle(handle, 'run')
  console.log('')
  console.log('  \x1b[32m✓\x1b[0m fireline ready')
  console.log('')
  console.log(`    sandbox:   ${handle.id}`)
  console.log(`    ACP:       ${handle.acp.url}`)
  console.log(`    state:     ${handle.state.url}`)
  if (args.stateStream) console.log(`    stream:    ${args.stateStream}`)
  console.log('')
  console.log('  Press Ctrl+C to shut down.')
  console.log('')
}

function printBuildResult(imageTag: string, scaffoldedFiles: readonly string[]): void {
  console.log('')
  console.log('  \x1b[32m✓\x1b[0m fireline build complete')
  console.log('')
  console.log(`    image:     ${imageTag}`)
  for (const filePath of scaffoldedFiles) {
    console.log(`    scaffold:  ${filePath}`)
  }
  console.log('')
}

function printDeployResult(imageTag: string, target: DeployTarget): void {
  console.log('')
  console.log('  \x1b[32m✓\x1b[0m fireline deploy complete')
  console.log('')
  console.log(`    image:     ${imageTag}`)
  console.log(`    target:    ${target}`)
  console.log('')
}

async function executeHostedBuild(
  args: ParsedArgs,
  runtime: CliRuntime,
  options: {
    readonly scaffoldTarget?: BuildTarget | null
    readonly scaffoldCwd?: string
    readonly printResult?: boolean
  } = {},
): Promise<HostedBuildResult> {
  const specPath = resolvePath(runtime.cwd(), args.file!)
  const spec = await runtime.loadSpec(specPath)
  const effectiveSpec = materializeSpec(spec, args)
  const appName = defaultAppName(specPath, effectiveSpec.name)
  const imageTag = defaultImageTag(appName)

  const dockerfile = findWorkspacePath('docker/fireline-host.Dockerfile')
  const buildContext = dirname(dirname(dockerfile))
  const plan = createDockerBuildPlan({
    buildContext,
    dockerfile,
    imageTag,
    spec: effectiveSpec,
  })

  const scaffoldTarget = options.scaffoldTarget === undefined ? args.target : options.scaffoldTarget
  const scaffoldPlan = scaffoldTarget
    ? createTargetScaffoldPlan({
        target: scaffoldTarget,
        cwd: options.scaffoldCwd ?? runtime.cwd(),
        appName,
        imageTag,
      })
    : null

  console.log(`fireline: building ${plan.imageTag}`)
  const exitCode = await runtime.runChild(plan.command, plan.args, { cwd: buildContext })
  if (exitCode !== 0) {
    return {
      exitCode,
      imageTag,
      scaffoldPlan,
    }
  }

  const scaffoldedFiles: string[] = []
  if (scaffoldPlan) {
    await runtime.writeTargetScaffold(scaffoldPlan)
    scaffoldedFiles.push(scaffoldPlan.filePath)
  }

  if (options.printResult ?? true) {
    printBuildResult(plan.imageTag, scaffoldedFiles)
  }

  return {
    exitCode: 0,
    imageTag,
    scaffoldPlan,
  }
}

export function createDeployExecutionPlan(options: {
  readonly target: DeployTarget
  readonly cwd: string
  readonly imageTag: string
  readonly scaffoldPath: string | null
  readonly passthroughArgs: readonly string[]
}): DeployExecutionPlan {
  const scaffoldPath = requireScaffoldPath(options.target, options.scaffoldPath)
  switch (options.target) {
    case 'cloudflare-containers':
      return {
        target: options.target,
        command: 'wrangler',
        args: ['deploy', '--config', scaffoldPath, ...options.passthroughArgs],
        cwd: options.cwd,
        installHint: 'Install Wrangler: https://developers.cloudflare.com/workers/wrangler/install-and-update/',
      }
    case 'docker-compose':
      return {
        target: options.target,
        command: 'docker',
        args: ['compose', '-f', scaffoldPath, 'up', '-d', ...options.passthroughArgs],
        cwd: options.cwd,
        installHint: 'Install Docker Engine or Docker Desktop with the Compose plugin: https://docs.docker.com/compose/install/',
      }
    case 'fly':
      return {
        target: options.target,
        command: 'flyctl',
        args: ['deploy', '--config', scaffoldPath, '--image', options.imageTag, ...options.passthroughArgs],
        cwd: options.cwd,
        installHint: 'Install flyctl: https://fly.io/docs/flyctl/install/',
      }
    case 'k8s':
      return {
        target: options.target,
        command: 'kubectl',
        args: ['apply', '-f', scaffoldPath, ...options.passthroughArgs],
        cwd: options.cwd,
        installHint: 'Install kubectl: https://kubernetes.io/docs/tasks/tools/',
      }
  }
}

function requireScaffoldPath(target: DeployTarget, scaffoldPath: string | null): string {
  if (!scaffoldPath) {
    throw new Error(`deploy target '${target}' requires a generated manifest`)
  }
  return scaffoldPath
}

function decorateMissingDeployToolError(plan: DeployExecutionPlan, error: unknown): Error {
  const message = (error as Error)?.message ?? String(error)
  return new Error(`${message}\nInstall ${plan.command} and retry.\n${plan.installHint}`)
}
