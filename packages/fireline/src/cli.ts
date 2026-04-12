import { ChildProcess, spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
import { writeFile } from 'node:fs/promises'
import { basename, dirname, parse as parsePath, resolve as resolvePath } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { tsImport } from 'tsx/esm/api'
import { resolveBinary } from './resolve-binary.js'

export type BuildTarget = 'cloudflare' | 'docker' | 'fly' | 'k8s'

export interface ParsedArgs {
  readonly command: 'run' | 'build' | 'help'
  readonly helpFor: 'general' | 'run' | 'build'
  readonly file: string | null
  readonly port: number
  readonly streamsPort: number
  readonly stateStream: string | null
  readonly name: string | null
  readonly repl: boolean
  readonly providerOverride: string | null
  readonly target: BuildTarget | null
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

const GENERAL_HELP = `
fireline — run specs locally or build hosted images

Usage:
  fireline [run] <file.ts>           Boot conductor + streams, provision agent locally
  fireline build <file.ts>           Build hosted Fireline OCI image
  fireline --help                    Show this help

Run flags:
  --port <n>           ACP control-plane port (default: 4440)
  --streams-port <n>   Durable-streams port   (default: 7474)
  --state-stream <s>   Explicit durable state stream name (enables resume)
  --name <s>           Logical agent name     (default: from spec or 'default')
  --provider <p>       Override sandbox.provider from spec
  --repl               Print ACP URL and wait (TODO: interactive REPL)

Build flags:
  --target <platform>  Scaffold target config: cloudflare | fly | docker | k8s
  --state-stream <s>   Override durable state stream name baked into the spec
  --name <s>           Override deployment name baked into the spec
  --provider <p>       Override sandbox.provider baked into the spec

Env:
  FIRELINE_BIN          Override path to fireline binary
  FIRELINE_STREAMS_BIN  Override path to fireline-streams binary

Example:
  fireline run examples/code-review-agent/index.ts
  fireline build agent.ts --target fly
`.trim()

const RUN_HELP = `
fireline run — boot Fireline locally and provision a spec

Usage:
  fireline [run] <file.ts> [flags]

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
  --target <platform>  Scaffold target config: cloudflare | fly | docker | k8s
  --state-stream <s>   Override durable state stream name baked into the spec
  --name <s>           Override deployment name baked into the spec
  --provider <p>       Override sandbox.provider baked into the spec
  --help               Show this help
`.trim()

export async function main(argv: readonly string[]): Promise<void> {
  let exitCode = 0
  try {
    const args = parseArgs(argv)
    if (args.command === 'help' || !args.file) {
      console.log(helpText(args.helpFor))
      return
    }
    exitCode = args.command === 'build'
      ? await build(args)
      : await run(args)
  } catch (error) {
    console.error(`fireline: ${(error as Error).message}`)
    exitCode = 1
  }
  process.exit(exitCode)
}

export function parseArgs(argv: readonly string[]): ParsedArgs {
  const out = {
    command: 'run' as 'run' | 'build' | 'help',
    helpFor: 'run' as 'general' | 'run' | 'build',
    file: null as string | null,
    port: 4440,
    streamsPort: 7474,
    stateStream: null as string | null,
    name: null as string | null,
    repl: false,
    providerOverride: null as string | null,
    target: null as BuildTarget | null,
  }
  const seen = {
    port: false,
    streamsPort: false,
    repl: false,
    target: false,
  }
  let i = 0
  if (argv[0] === 'run' || argv[0] === 'build') {
    out.command = argv[0]
    out.helpFor = argv[0]
    i++
  }
  if (argv[0] === '--help' || argv[0] === '-h') {
    return { ...out, command: 'help', helpFor: 'general' }
  }

  for (; i < argv.length; i++) {
    const arg = argv[i]
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
  if (out.command === 'build' && seen.port) {
    throw new Error('--port is only valid with run')
  }
  if (out.command === 'build' && seen.streamsPort) {
    throw new Error('--streams-port is only valid with run')
  }
  if (out.command === 'build' && seen.repl) {
    throw new Error('--repl is only valid with run')
  }

  return out
}

function helpText(topic: ParsedArgs['helpFor']): string {
  switch (topic) {
    case 'run':
      return RUN_HELP
    case 'build':
      return BUILD_HELP
    case 'general':
      return GENERAL_HELP
  }
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
    case 'fly':
    case 'flyio':
      return 'fly'
    case 'k8s':
    case 'kubernetes':
      return 'k8s'
    default:
      throw new Error(`unsupported build target: ${normalized} (expected cloudflare, fly, docker, or k8s)`)
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

async function build(args: ParsedArgs): Promise<number> {
  const specPath = resolvePath(process.cwd(), args.file!)
  const spec = await loadSpec(specPath)
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

  const scaffoldPlan = args.target
    ? createTargetScaffoldPlan({
        target: args.target,
        cwd: process.cwd(),
        appName,
        imageTag,
      })
    : null

  console.log(`fireline: building ${plan.imageTag}`)
  const exitCode = await runChild(plan.command, plan.args, { cwd: buildContext })
  if (exitCode !== 0) return exitCode

  const scaffoldedFiles: string[] = []
  if (scaffoldPlan) {
    await writeTargetScaffold(scaffoldPlan)
    scaffoldedFiles.push(scaffoldPlan.filePath)
  }

  printBuildResult(plan.imageTag, scaffoldedFiles)
  return 0
}

interface LoadedSpec {
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
