import { ChildProcess, spawn } from 'node:child_process'
import { dirname, resolve as resolvePath } from 'node:path'
import { pathToFileURL } from 'node:url'
import { tsImport } from 'tsx/esm/api'
import { resolveBinary } from './resolve-binary.js'

interface ParsedArgs {
  readonly command: 'run' | 'deploy' | 'help'
  readonly helpFor: 'general' | 'run' | 'deploy'
  readonly file: string | null
  readonly port: number
  readonly streamsPort: number
  readonly stateStream: string | null
  readonly name: string | null
  readonly repl: boolean
  readonly providerOverride: string | null
  readonly remote: string | null
  readonly token: string | null
}

const GENERAL_HELP = `
fireline — run or deploy declarative agent specs

Usage:
  fireline [run] <file.ts>           Boot conductor + streams, provision agent locally
  fireline deploy <file.ts>          Push spec to a remote Fireline instance
  fireline --help                    Show this help

Run flags:
  --port <n>           ACP control-plane port (default: 4440)
  --streams-port <n>   Durable-streams port   (default: 7474)
  --state-stream <s>   Explicit durable state stream name (enables resume)
  --name <s>           Logical agent name     (default: from spec or 'default')
  --provider <p>       Override sandbox.provider from spec
  --repl               Print ACP URL and wait (TODO: interactive REPL)

Deploy flags:
  --remote <url>       Hosted Fireline base URL (required)
  --token <token>      Bearer token for the remote instance
  --state-stream <s>   Explicit durable state stream name
  --name <s>           Logical deployment name (default: from spec or 'default')
  --provider <p>       Override sandbox.provider from spec
  --repl               Print ACP URL and wait (TODO: interactive REPL)

Env:
  FIRELINE_BIN          Override path to fireline binary
  FIRELINE_STREAMS_BIN  Override path to fireline-streams binary

Example:
  fireline run examples/code-review-agent/index.ts
  fireline deploy agent.ts --remote https://agents.example.com
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

const DEPLOY_HELP = `
fireline deploy — push a spec to a remote Fireline instance

Usage:
  fireline deploy <file.ts> --remote <url> [flags]

Flags:
  --remote <url>       Hosted Fireline base URL (required)
  --token <token>      Bearer token for the remote instance
  --state-stream <s>   Explicit durable state stream name
  --name <s>           Logical deployment name (default: from spec or 'default')
  --provider <p>       Override sandbox.provider from spec
  --repl               Print ACP URL and wait (TODO: interactive REPL)
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
    exitCode = args.command === 'deploy'
      ? await deploy(args)
      : await run(args)
  } catch (error) {
    console.error(`fireline: ${(error as Error).message}`)
    exitCode = 1
  }
  process.exit(exitCode)
}

export function parseArgs(argv: readonly string[]): ParsedArgs {
  const out = {
    command: 'run' as 'run' | 'deploy' | 'help',
    helpFor: 'run' as 'general' | 'run' | 'deploy',
    file: null as string | null,
    port: 4440,
    streamsPort: 7474,
    stateStream: null as string | null,
    name: null as string | null,
    repl: false,
    providerOverride: null as string | null,
    remote: null as string | null,
    token: null as string | null,
  }
  let i = 0
  if (argv[0] === 'run' || argv[0] === 'deploy') {
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
        out.port = parseIntArg(argv[++i], '--port')
        break
      case '--streams-port':
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
      case '--remote':
        out.remote = required(argv[++i], '--remote')
        break
      case '--token':
        out.token = required(argv[++i], '--token')
        break
      case '--repl':
        out.repl = true
        break
      default:
        if (arg?.startsWith('--')) throw new Error(`unknown flag: ${arg}`)
        if (out.file) throw new Error(`unexpected argument: ${arg}`)
        out.file = arg
    }
  }

  if (out.command === 'deploy' && !out.remote) {
    throw new Error('deploy requires --remote <url>')
  }
  if (out.command === 'run' && out.remote) {
    throw new Error('--remote is only valid with deploy')
  }
  if (out.command === 'run' && out.token) {
    throw new Error('--token is only valid with deploy')
  }

  return out
}

function helpText(topic: ParsedArgs['helpFor']): string {
  switch (topic) {
    case 'run':
      return RUN_HELP
    case 'deploy':
      return DEPLOY_HELP
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
    // 1. Start durable-streams
    const streamsProc = spawn(streamsBin, [], {
      stdio: ['ignore', 'inherit', 'inherit'],
      env: { ...process.env, PORT: String(args.streamsPort) },
    })
    teardown.push(() => stopChild(streamsProc))
    await waitForHttp(`http://127.0.0.1:${args.streamsPort}/healthz`, 10_000, 'fireline-streams')

    // 2. Start control plane
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

    // 3. Provision the agent via spec.start()
    const startOptions: Record<string, unknown> = {
      serverUrl: `http://127.0.0.1:${args.port}`,
    }
    if (args.stateStream) startOptions.stateStream = args.stateStream
    if (args.name) startOptions.name = args.name

    // If --provider override is set, mutate the spec before starting.
    const effectiveSpec = args.providerOverride
      ? { ...spec, sandbox: { ...spec.sandbox, provider: args.providerOverride } }
      : spec

    const handle = await effectiveSpec.start(startOptions)
    teardown.push(() => destroySandbox(handle.id, args.port))

    printReady(handle, args)
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

interface DeployRequestBody {
  readonly name: string
  readonly spec: Record<string, unknown>
}

async function deploy(args: ParsedArgs): Promise<number> {
  const specPath = resolvePath(process.cwd(), args.file!)
  const spec = await loadSpec(specPath)
  const effectiveSpec = materializeSpec(spec, args)
  const remoteBaseUrl = args.remote!.replace(/\/+$/, '')

  const response = await fetch(`${remoteBaseUrl}/v1/deployments`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      ...(args.token ? { authorization: `Bearer ${args.token}` } : {}),
    },
    body: JSON.stringify({
      name: effectiveSpec.name,
      spec: effectiveSpec,
    } satisfies DeployRequestBody),
    signal: AbortSignal.timeout(15_000),
  })

  if (!response.ok) {
    const detail = await readResponseDetail(response)
    throw new Error(
      `deploy failed (${response.status} ${response.statusText})` +
        (detail ? `: ${detail}` : ''),
    )
  }

  const handle = await response.json() as PrintedHandle
  validatePrintedHandle(handle, 'deploy')
  printReady(handle, args)
  if (args.repl) {
    console.log('\nREPL mode coming soon. Connect any ACP client to the URL above.')
  }
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
  // tsImport's specifier must be a path relative to parentURL, or an absolute
  // file URL resolved via parentURL pointing at the same directory.
  const parentURL = pathToFileURL(`${dirname(specPath)}/`).href
  const mod = await tsImport(`./${specPath.split('/').pop()}`, parentURL)
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

function sleep(ms: number): Promise<void> {
  return new Promise((resolveWait) => setTimeout(resolveWait, ms))
}

interface PrintedHandle {
  readonly id: string
  readonly acp: { readonly url: string }
  readonly state: { readonly url: string }
}

interface SerializedHarnessSpec extends Record<string, unknown> {
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

async function readResponseDetail(response: Response): Promise<string> {
  const text = await response.text()
  return text.trim()
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
  console.log('')
  console.log('  \x1b[32m✓\x1b[0m fireline ready')
  console.log('')
  console.log(`    ${args.command === 'deploy' ? 'deployment' : 'sandbox'}:   ${handle.id}`)
  console.log(`    ACP:       ${handle.acp.url}`)
  console.log(`    state:     ${handle.state.url}`)
  if (args.stateStream) console.log(`    stream:    ${args.stateStream}`)
  console.log('')
  console.log('  Press Ctrl+C to shut down.')
  console.log('')
}
