import { ChildProcess, spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, parse as parsePath, relative as relativePath, resolve as resolvePath, sep as pathSep } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { tsImport } from 'tsx/esm/api'
import {
  createDeployExecutionPlan,
  createTargetScaffoldPlan,
  decorateMissingDeployToolError,
  hostedDockerfileForTarget,
  writeTargetScaffold,
} from './deploy/index.js'
import type {
  BuildTarget,
  DeployTarget,
  SerializedHarnessSpec,
  TargetScaffoldPlan,
} from './deploy/index.js'
import {
  BinaryResolutionError,
  type ResolvedBinary,
  findBinary,
  resolveBinary,
} from './resolve-binary.js'
import {
  loadHostedConfig,
  parseDeployTarget,
  resolveHostedDeploy,
} from './hosted-config.js'
import { probeExistingHostForRepl } from './host-probe.js'
import { runRepl } from './repl.js'

export {
  createDeployExecutionPlan,
  createTargetScaffoldPlan,
  writeTargetScaffold,
} from './deploy/index.js'
export type {
  BuildTarget,
  DeployExecutionPlan,
  DeployTarget,
  SerializedHarnessSpec,
  TargetScaffoldPlan,
} from './deploy/index.js'

export interface ParsedArgs {
  readonly command: 'run' | 'build' | 'deploy' | 'agents' | 'repl' | 'help'
  readonly helpFor: 'general' | 'run' | 'build' | 'deploy' | 'agents' | 'repl'
  readonly file: string | null
  readonly passthroughArgs: readonly string[]
  readonly port: number
  readonly repl: boolean
  readonly streamsPort: number
  readonly stateStream: string | null
  readonly name: string | null
  readonly providerOverride: string | null
  readonly sessionId: string | null
  readonly target: BuildTarget | null
  readonly targetName: string | null
  readonly token: string | null
  readonly to: DeployTarget | null
}

export interface DockerBuildPlan {
  readonly command: 'docker'
  readonly args: readonly string[]
  readonly buildArg: string
  readonly buildContext: string
  readonly dockerfile: string
  readonly embeddedSpecPath: string
  readonly embeddedSpecRelativePath: string
  readonly imageTag: string
}

interface HostedBuildResult {
  readonly exitCode: number
  readonly appName: string
  readonly imageTag: string
  readonly scaffoldPlan: TargetScaffoldPlan | null
}

export interface CliRuntime {
  readonly cwd: () => string
  readonly loadSpec: (specPath: string) => Promise<LoadedSpec>
  readonly loadHostedConfig: (configPath?: string) => Promise<import('./hosted-config.js').HostedConfig | null>
  readonly runChild: (
    command: string,
    args: readonly string[],
    options?: { readonly cwd?: string; readonly env?: NodeJS.ProcessEnv },
  ) => Promise<number>
  readonly writeTargetScaffold: (plan: TargetScaffoldPlan) => Promise<void>
}

const defaultCliRuntime: CliRuntime = {
  cwd: () => invocationCwd(),
  loadSpec,
  loadHostedConfig,
  runChild,
  writeTargetScaffold,
}

const WORKSPACE_ROOT = resolvePath(dirname(fileURLToPath(import.meta.url)), '../../..')
const SPEC_LOADER_TSCONFIG = resolvePath(WORKSPACE_ROOT, 'packages/fireline/tsconfig.loader.json')
const CLIENT_DIST_ENTRY = resolvePath(WORKSPACE_ROOT, 'packages/client/dist/index.js')
const STATE_DIST_ENTRY = resolvePath(WORKSPACE_ROOT, 'packages/state/dist/index.js')

const GENERAL_HELP = `
fireline — run specs locally, build hosted images, deploy them, or install ACP agents

Usage:
  fireline run <file.ts> [flags]     Boot conductor + streams, provision agent locally
  fireline <file.ts> [flags]         Shorthand for run
  fireline build <file.ts> [flags]   Build hosted Fireline OCI image
  fireline deploy <file.ts> [--to <platform> | --target <name>] [-- <native-flags...>]
  fireline repl [session-id]         Connect to a running Fireline ACP host
  fireline agents <command> [args]   Install ACP agents from the public registry
  fireline --help                    Show this help

Run flags:
  --port <n>           ACP control-plane port (default: 4440)
  --repl               Start an interactive REPL after booting the host
  --streams-port <n>   Durable-streams port   (default: 7474)
  --state-stream <s>   Explicit durable state stream name (enables resume)
  --name <s>           Logical agent name     (default: from spec or 'default')
  --provider <p>       Override sandbox.provider from spec

Build flags:
  --target <platform>  Scaffold target config: cloudflare | docker | docker-compose | fly | k8s
  --state-stream <s>   Override durable state stream name baked into the spec
  --name <s>           Override deployment name baked into the spec
  --provider <p>       Override sandbox.provider baked into the spec

Deploy flags:
  --to <platform>      Native deploy target: fly | cloudflare-containers | docker-compose | k8s
  --target <name>      Hosted target from ~/.fireline/hosted.json
  --token <value>      Override deploy auth token for the resolved target
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
  fireline deploy agent.ts --target production
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
  --repl               Start an interactive REPL after booting the host
  --streams-port <n>   Durable-streams port   (default: 7474)
  --state-stream <s>   Explicit durable state stream name (enables resume)
  --name <s>           Logical agent name     (default: from spec or 'default')
  --provider <p>       Override sandbox.provider from spec
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
  fireline deploy <file.ts> [--to <platform> | --target <name>] [flags] [-- <native-flags...>]

Flags:
  --to <platform>      Native deploy target: fly | cloudflare-containers | docker-compose | k8s
  --target <name>      Hosted target from ~/.fireline/hosted.json
  --token <value>      Override deploy auth token for the resolved target
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
  fireline deploy agent.ts --target production
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

const REPL_HELP = `
fireline repl — interactive ACP client for a running Fireline host

Usage:
  fireline repl
  fireline repl <session-id>

Env:
  FIRELINE_URL          Fireline host URL (default: http://127.0.0.1:4440)

Examples:
  fireline repl
  fireline repl session-123
`.trim()

export async function main(argv: readonly string[]): Promise<void> {
  let exitCode = 0
  try {
    const args = parseArgs(argv)
    if (
      args.command === 'help' ||
      ((args.command === 'run' || args.command === 'build' || args.command === 'deploy') &&
        !args.file)
    ) {
      console.log(helpText(args.helpFor))
      return
    }
    exitCode = args.command === 'build'
      ? await build(args)
      : args.command === 'deploy'
        ? await deploy(args)
        : args.command === 'repl'
          ? await runReplCommand(args)
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
    command: 'run' as 'run' | 'build' | 'deploy' | 'agents' | 'repl' | 'help',
    helpFor: 'run' as 'general' | 'run' | 'build' | 'deploy' | 'agents' | 'repl',
    file: null as string | null,
    passthroughArgs: [] as string[],
    port: 4440,
    repl: false,
    streamsPort: 7474,
    stateStream: null as string | null,
    name: null as string | null,
    providerOverride: null as string | null,
    sessionId: null as string | null,
    target: null as BuildTarget | null,
    targetName: null as string | null,
    token: null as string | null,
    to: null as DeployTarget | null,
  }
  const seen = {
    port: false,
    streamsPort: false,
    to: false,
  }
  let i = 0
  if (
    argv[0] === 'run' ||
    argv[0] === 'build' ||
    argv[0] === 'deploy' ||
    argv[0] === 'agents' ||
    argv[0] === 'repl'
  ) {
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
    if (out.command === 'repl') {
      if (arg === '--help' || arg === '-h') {
        return { ...out, command: 'help', helpFor: 'repl' }
      }
      if (arg === '--repl') {
        throw new Error("--repl has been replaced by 'fireline repl [session-id]'")
      }
      if (arg?.startsWith('--')) {
        throw new Error(`unknown flag: ${arg}`)
      }
      if (out.sessionId) {
        throw new Error(`unexpected argument: ${arg}`)
      }
      out.sessionId = arg
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
      case '--repl':
        out.repl = true
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
        if (out.command === 'build') {
          out.target = parseBuildTarget(argv[++i])
          break
        }
        if (out.command === 'deploy') {
          out.targetName = required(argv[++i], '--target')
          break
        }
        throw new Error('--target is only valid with build or deploy')
      case '--token':
        if (out.command !== 'deploy') {
          throw new Error('--token is only valid with deploy')
        }
        out.token = required(argv[++i], '--token')
        break
      case '--to':
        seen.to = true
        out.to = parseDeployTarget(argv[++i])
        break
      default:
        if (arg?.startsWith('--')) throw new Error(`unknown flag: ${arg}`)
        if (out.file) throw new Error(`unexpected argument: ${arg}`)
        out.file = arg
    }
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
  if (out.command === 'build' && seen.to) {
    throw new Error('--to is only valid with deploy')
  }
  if (out.command === 'build' && out.repl) {
    throw new Error('--repl is only valid with run')
  }
  if (out.command === 'deploy' && seen.port) {
    throw new Error('--port is only valid with run')
  }
  if (out.command === 'deploy' && seen.streamsPort) {
    throw new Error('--streams-port is only valid with run')
  }
  if (out.command === 'deploy' && out.repl) {
    throw new Error('--repl is only valid with run')
  }

  return out
}

export function helpText(topic: ParsedArgs['helpFor']): string {
  switch (topic) {
    case 'run':
      return RUN_HELP
    case 'build':
      return BUILD_HELP
    case 'deploy':
      return DEPLOY_HELP
    case 'agents':
      return AGENTS_HELP
    case 'repl':
      return REPL_HELP
    case 'general':
      return GENERAL_HELP
  }
}

export async function runReplCommand(
  args: ParsedArgs,
  replRunner: typeof runRepl = runRepl,
): Promise<number> {
  if (args.sessionId && looksLikeSpecPath(args.sessionId)) {
    throw new Error(
      `${args.sessionId} looks like a spec path, not a session id. Did you mean: fireline run ${args.sessionId} --repl ?`,
    )
  }

  try {
    return await replRunner({ sessionId: args.sessionId })
  } catch (error) {
    throw decorateStandaloneReplError(error)
  }
}

export async function runAgents(args: ParsedArgs): Promise<number> {
  const agentsBin = resolveBinary({ name: 'fireline-agents', envVar: 'FIRELINE_AGENTS_BIN' })
  return await runChild(agentsBin.path, args.passthroughArgs)
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

export async function run(args: ParsedArgs): Promise<number> {
  if (args.repl) {
    const started = await startHostedRunSession(args)
    try {
      return await runHostedRepl(started.handle, args, runRepl, console, {
        sessionId: started.sessionId,
      })
    } finally {
      await started.close()
    }
  }

  const specPath = resolvePath(invocationCwd(), args.file!)
  const spec = await loadSpec(specPath)

  const { firelineBin, streamsBin } = resolveRunBinaries()
  maybeLogBinaryMismatch(firelineBin, streamsBin)

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
    const streamsHealthz = `http://127.0.0.1:${args.streamsPort}/healthz`
    const reusingStreams = await isHealthy(streamsHealthz)
    if (reusingStreams) {
      console.log(`fireline: reusing fireline-streams at :${args.streamsPort}`)
    } else {
      const streamsProc = spawn(streamsBin.path, [], {
        stdio: ['ignore', 'inherit', 'inherit'],
        env: { ...process.env, PORT: String(args.streamsPort) },
      })
      teardown.push(() => stopChild(streamsProc))
      await waitForHttp(streamsHealthz, 10_000, 'fireline-streams')
    }

    const hostHealthz = `http://127.0.0.1:${args.port}/healthz`
    if (await isHealthy(hostHealthz)) {
      throw new Error(`Port ${args.port} already in use; stop the other process or pass --port <n>`)
    }
    const controlPlaneArgs = [
      '--control-plane',
      '--port', String(args.port),
      '--durable-streams-url', `http://127.0.0.1:${args.streamsPort}/v1/stream`,
    ]
    const firelineProc = spawn(firelineBin.path, controlPlaneArgs, {
      stdio: ['ignore', 'inherit', 'inherit'],
      env: { ...process.env },
    })
    teardown.push(() => stopChild(firelineProc))
    await waitForHttp(hostHealthz, 15_000, 'fireline')

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
    printInteractionHint(args.file!)

    const signalCode = await waitForShutdown
    await runTeardown()
    return signalCode
  } catch (error) {
    await runTeardown()
    throw error
  }
}

export interface StartedHostedRunSession {
  readonly handle: PrintedHandle
  readonly sessionId: string | null
  close(): Promise<void>
}

export async function startHostedRunSession(
  args: ParsedArgs,
): Promise<StartedHostedRunSession> {
  const specPath = resolvePath(invocationCwd(), args.file!)
  const spec = await loadSpec(specPath)

  const { firelineBin, streamsBin } = resolveRunBinaries()
  maybeLogBinaryMismatch(firelineBin, streamsBin)

  const teardown: Array<() => Promise<void> | void> = []

  async function runTeardown(): Promise<void> {
    for (const fn of teardown.reverse()) {
      try {
        await fn()
      } catch (error) {
        console.error(`fireline: teardown error: ${(error as Error).message}`)
      }
    }
  }

  try {
    const streamsHealthz = `http://127.0.0.1:${args.streamsPort}/healthz`
    const reusingStreams = await isHealthy(streamsHealthz)
    if (reusingStreams) {
      console.log(`fireline: reusing fireline-streams at :${args.streamsPort}`)
    } else {
      const streamsProc = spawn(streamsBin.path, [], {
        stdio: ['ignore', 'inherit', 'inherit'],
        env: { ...process.env, PORT: String(args.streamsPort) },
      })
      teardown.push(() => stopChild(streamsProc))
      await waitForHttp(streamsHealthz, 10_000, 'fireline-streams')
    }

    const hostHealthz = `http://127.0.0.1:${args.port}/healthz`
    if (await isHealthy(hostHealthz)) {
      const existingHost = await probeExistingHostForRepl(`http://127.0.0.1:${args.port}`)
      switch (existingHost.kind) {
        case 'attachable':
          console.log(`fireline: reusing fireline host at :${args.port}`)
          return {
            handle: printedHandleFromExistingHost(existingHost.handle),
            sessionId: existingHost.latestSessionId ?? null,
            close: runTeardown,
          }
        case 'multiple-live-sandboxes':
          throw new Error(
            `Port ${args.port} already hosts Fireline with ${existingHost.count} live sandboxes; auto-attach is ambiguous. Use 'fireline repl <session-id>' or pass --port <n>.`,
          )
        case 'no-live-sandboxes':
          throw new Error(
            `Port ${args.port} already hosts Fireline, but no live sandboxes are available to attach. Stop the other process or pass --port <n>.`,
          )
        case 'not-fireline':
          throw new Error(`Port ${args.port} already in use; stop the other process or pass --port <n>`)
      }
    }

    const controlPlaneArgs = [
      '--control-plane',
      '--port', String(args.port),
      '--durable-streams-url', `http://127.0.0.1:${args.streamsPort}/v1/stream`,
    ]
    const firelineProc = spawn(firelineBin.path, controlPlaneArgs, {
      stdio: ['ignore', 'inherit', 'inherit'],
      env: { ...process.env },
    })
    teardown.push(() => stopChild(firelineProc))
    await waitForHttp(hostHealthz, 15_000, 'fireline')

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

    return {
      handle: agentHandle,
      sessionId: null,
      close: runTeardown,
    }
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
  const hostedConfig = await runtime.loadHostedConfig()
  const hosted = resolveHostedDeploy({
    config: hostedConfig,
    targetName: args.targetName,
    deployTarget: args.to,
    token: args.token,
    env: process.env,
  })
  const effectiveArgs: ParsedArgs = {
    ...args,
    name: args.name ?? hosted.target?.resourceNaming?.appName ?? null,
    to: hosted.deployTarget,
  }

  const scaffoldCwd = await mkdtemp(resolvePath(tmpdir(), 'fireline-deploy-'))
  try {
    const buildResult = await executeHostedBuild(effectiveArgs, runtime, {
      scaffoldTarget: deployScaffoldTarget(hosted.deployTarget),
      scaffoldCwd,
      printResult: false,
    })
    if (buildResult.exitCode !== 0) return buildResult.exitCode

    const nativePlan = createDeployExecutionPlan({
      target: hosted.deployTarget,
      cwd: runtime.cwd(),
      imageTag: buildResult.imageTag,
      scaffoldPath: buildResult.scaffoldPlan?.filePath ?? null,
      passthroughArgs: args.passthroughArgs,
      appName: buildResult.appName,
    })

    if (hosted.targetName) {
      console.log(`fireline: resolved hosted target ${hosted.targetName} -> ${hosted.deployTarget}`)
    }
    console.log(`fireline: deploying ${buildResult.imageTag} via ${nativePlan.command}`)
    const childEnv = hosted.token
      ? {
          ...process.env,
          [hosted.tokenSinkEnvVar]: hosted.token,
        }
      : undefined
    for (const step of nativePlan.steps) {
      try {
        const exitCode = await runtime.runChild(step.command, step.args, {
          cwd: step.cwd,
          env: childEnv,
        })
        if (exitCode !== 0) {
          return exitCode
        }
      } catch (error) {
        throw decorateMissingDeployToolError(step, error)
      }
    }

    printDeployResult(buildResult.imageTag, hosted.deployTarget)
    return 0
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

export async function loadSpec(specPath: string): Promise<LoadedSpec> {
  await ensureWorkspaceSpecDependenciesBuilt(specPath)
  const parentURL = pathToFileURL(`${dirname(specPath)}/`).href
  const mod = await tsImport(pathToFileURL(specPath).href, {
    parentURL,
    ...(existsSync(SPEC_LOADER_TSCONFIG) ? { tsconfig: SPEC_LOADER_TSCONFIG } : {}),
  })
  const candidate = unwrapDefaultExport((mod as { default?: unknown }).default ?? mod)
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

export function unwrapDefaultExport(value: unknown): unknown {
  let current = value
  const seen = new Set<object>()
  while (
    current &&
    typeof current === 'object' &&
    typeof (current as { start?: unknown }).start !== 'function' &&
    'default' in (current as Record<string, unknown>)
  ) {
    if (seen.has(current as object)) {
      break
    }
    seen.add(current as object)
    current = (current as { default?: unknown }).default
  }
  return current
}

async function ensureWorkspaceSpecDependenciesBuilt(specPath: string): Promise<void> {
  if (!specPath.startsWith(WORKSPACE_ROOT)) {
    return
  }

  const source = await readFile(specPath, 'utf8')
  const needsClient = source.includes('@fireline/client')
  const needsState = source.includes('@fireline/state')

  if ((needsClient || needsState) && !existsSync(STATE_DIST_ENTRY)) {
    await buildWorkspacePackage('@fireline/state')
  }
  if (needsClient && !existsSync(CLIENT_DIST_ENTRY)) {
    await buildWorkspacePackage('@fireline/client')
  }
}

async function buildWorkspacePackage(filter: '@fireline/client' | '@fireline/state'): Promise<void> {
  const exitCode = await runChild('pnpm', ['--filter', filter, 'build'], { cwd: WORKSPACE_ROOT })
  if (exitCode !== 0) {
    throw new Error(`failed to build ${filter} for repo-local spec loading`)
  }
}

async function isHealthy(url: string): Promise<boolean> {
  try {
    const res = await fetch(url, { signal: AbortSignal.timeout(500) })
    return res.ok
  } catch {
    return false
  }
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
  options: { readonly cwd?: string; readonly env?: NodeJS.ProcessEnv } = {},
): Promise<number> {
  const child = spawn(command, [...args], {
    cwd: options.cwd,
    stdio: ['ignore', 'inherit', 'inherit'],
    env: options.env ?? { ...process.env },
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

function invocationCwd(): string {
  return process.env.PWD || process.cwd()
}

function resolveRunBinaries(): {
  firelineBin: ResolvedBinary
  streamsBin: ResolvedBinary
} {
  const firelineBin = tryFindRunBinary({ name: 'fireline', envVar: 'FIRELINE_BIN' })
  const streamsBin = tryFindRunBinary({
    name: 'fireline-streams',
    envVar: 'FIRELINE_STREAMS_BIN',
  })
  const missing = [firelineBin, streamsBin].filter((entry) => entry === null)
  if (missing.length > 0) {
    throw new Error(
      [
        'Could not find required Fireline binaries.',
        'Run exactly:',
        '  cargo build --release --bin fireline --bin fireline-streams',
      ].join('\n'),
    )
  }
  return {
    firelineBin: firelineBin!,
    streamsBin: streamsBin!,
  }
}

function tryFindRunBinary(
  lookup: Parameters<typeof findBinary>[0],
): ResolvedBinary | null {
  try {
    return findBinary(lookup)
  } catch (error) {
    if (
      error instanceof BinaryResolutionError &&
      error.kind === 'env-missing'
    ) {
      throw error
    }
    throw error
  }
}

function maybeLogBinaryMismatch(
  firelineBin: ResolvedBinary,
  streamsBin: ResolvedBinary,
): void {
  if (
    (firelineBin.source !== 'release' && firelineBin.source !== 'debug') ||
    (streamsBin.source !== 'release' && streamsBin.source !== 'debug') ||
    firelineBin.source === streamsBin.source
  ) {
    return
  }
  console.info(
    `fireline: using ${firelineBin.name} from target/${firelineBin.source} and ${streamsBin.name} from target/${streamsBin.source}`,
  )
}

export interface PrintedHandle {
  readonly id: string
  readonly acp: { readonly url: string }
  readonly state: { readonly url: string }
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

export async function createDockerBuildPlan(options: {
  readonly buildContext: string
  readonly dockerfile: string
  readonly imageTag: string
  readonly spec: SerializedHarnessSpec
}): Promise<DockerBuildPlan> {
  const embeddedSpecRoot = resolvePath(options.buildContext, '.tmp')
  await mkdir(embeddedSpecRoot, { recursive: true })
  const embeddedSpecDir = await mkdtemp(resolvePath(embeddedSpecRoot, 'fireline-embedded-spec-'))
  const embeddedSpecPath = resolvePath(embeddedSpecDir, 'spec.json')
  await writeFile(embeddedSpecPath, `${JSON.stringify(options.spec, null, 2)}\n`)
  const embeddedSpecRelativePath = relativePath(options.buildContext, embeddedSpecPath).split(pathSep).join('/')
  const buildArg = `SPEC=${embeddedSpecRelativePath}`
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
    embeddedSpecPath,
    embeddedSpecRelativePath,
    imageTag: options.imageTag,
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

export async function runHostedRepl(
  handle: PrintedHandle,
  args: ParsedArgs,
  replRunner: typeof runRepl = runRepl,
  logger: ReadyLogger = console,
  options: {
    readonly sessionId?: string | null
  } = {},
): Promise<number> {
  validatePrintedHandle(handle, 'run')

  return await replRunner({
    acpUrl: handle.acp.url,
    onSessionReady: async (sessionId: string) => {
      printReady(handle, args, { logger, sessionId })
    },
    runtimeId: handle.id,
    serverUrl: `http://127.0.0.1:${args.port}`,
    sessionId: options.sessionId ?? null,
    stateStreamUrl: handle.state.url,
  })
}

function printedHandleFromExistingHost(handle: {
  readonly id: string
  readonly acp: { readonly url: string }
  readonly state: { readonly url: string }
}): PrintedHandle {
  return {
    id: handle.id,
    acp: handle.acp,
    state: handle.state,
  }
}

interface ReadyLogger {
  log: (...args: unknown[]) => void
}

function printReady(
  handle: PrintedHandle,
  args: ParsedArgs,
  options: {
    readonly logger?: ReadyLogger
    readonly sessionId?: string | null
  } = {},
): void {
  const logger = options.logger ?? console
  validatePrintedHandle(handle, 'run')
  logger.log('')
  logger.log('  \x1b[32m✓\x1b[0m fireline ready')
  logger.log('')
  logger.log(`    sandbox:   ${handle.id}`)
  logger.log(`    ACP:       ${handle.acp.url}`)
  logger.log(`    state:     ${handle.state.url}`)
  if (options.sessionId) logger.log(`    session:   ${options.sessionId}`)
  if (args.stateStream) logger.log(`    stream:    ${args.stateStream}`)
  logger.log('')
  logger.log('  Press Ctrl+C to shut down.')
  logger.log('')
}

function printInteractionHint(file: string, logger: ReadyLogger = console): void {
  logger.log(`  To interact: npx fireline ${file} --repl`)
  logger.log('')
}

export function looksLikeSpecPath(value: string): boolean {
  return /\.[cm]?[jt]sx?$/i.test(value)
}

export function decorateStandaloneReplError(error: unknown): Error {
  const failure = error instanceof Error ? error : new Error(String(error))
  if (!isLikelyMissingReplHost(failure.message)) {
    return failure
  }

  return new Error(
    `no fireline host running on ${formatReplServerLabel(process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440')}. Start one: fireline run <spec>`,
  )
}

function isLikelyMissingReplHost(message: string): boolean {
  return (
    message.includes('ECONNREFUSED') ||
    message.includes('Unexpected server response: 404') ||
    message.includes('Unexpected server response: 503') ||
    message.includes('socket hang up')
  )
}

function formatReplServerLabel(serverUrl: string): string {
  try {
    const url = new URL(serverUrl)
    if ((url.hostname === '127.0.0.1' || url.hostname === 'localhost') && url.port) {
      return `:${url.port}`
    }
    return url.origin
  } catch {
    return serverUrl
  }
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
  const scaffoldTarget = options.scaffoldTarget === undefined ? args.target : options.scaffoldTarget

  const dockerfile = findWorkspacePath(hostedDockerfileForTarget(scaffoldTarget ?? null))
  const buildContext = dirname(dirname(dockerfile))
  const plan = await createDockerBuildPlan({
    buildContext,
    dockerfile,
    imageTag,
    spec: effectiveSpec,
  })

  const scaffoldPlan = scaffoldTarget
    ? createTargetScaffoldPlan({
        target: scaffoldTarget,
        cwd: options.scaffoldCwd ?? runtime.cwd(),
        appName,
        imageTag,
      })
    : null

  console.log(`fireline: building ${plan.imageTag}`)
  try {
    const exitCode = await runtime.runChild(plan.command, plan.args, { cwd: buildContext })
    if (exitCode !== 0) {
      return {
        exitCode,
        appName,
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
      exitCode,
      appName,
      imageTag,
      scaffoldPlan,
    }
  } finally {
    if (plan.embeddedSpecPath) {
      await rm(dirname(plan.embeddedSpecPath), { recursive: true, force: true })
    }
  }
}

const invokedPath = process.argv[1] ? resolvePath(process.argv[1]) : null
if (invokedPath && invokedPath === fileURLToPath(import.meta.url)) {
  void main(process.argv.slice(2))
}
