import { ChildProcess, spawn } from 'node:child_process'
import { dirname, resolve as resolvePath } from 'node:path'
import { pathToFileURL } from 'node:url'
import { tsImport } from 'tsx/esm/api'
import { resolveBinary } from './resolve-binary.js'

interface ParsedArgs {
  readonly command: 'run' | 'help'
  readonly file: string | null
  readonly port: number
  readonly streamsPort: number
  readonly stateStream: string | null
  readonly name: string | null
  readonly repl: boolean
  readonly providerOverride: string | null
}

const HELP = `
fireline — run declarative agent specs

Usage:
  fireline [run] <file.ts>           Boot conductor + streams, provision agent
  fireline --help                    Show this help

Flags:
  --port <n>           ACP control-plane port (default: 4440)
  --streams-port <n>   Durable-streams port   (default: 7474)
  --state-stream <s>   Explicit durable state stream name (enables resume)
  --name <s>           Logical agent name     (default: from spec or 'default')
  --provider <p>       Override sandbox.provider from spec
  --repl               Print ACP URL and wait (TODO: interactive REPL)

Env:
  FIRELINE_BIN          Override path to fireline binary
  FIRELINE_STREAMS_BIN  Override path to fireline-streams binary

Example:
  fireline run examples/code-review-agent/index.ts
`.trim()

export async function main(argv: readonly string[]): Promise<void> {
  let exitCode = 0
  try {
    const args = parseArgs(argv)
    if (args.command === 'help' || !args.file) {
      console.log(HELP)
      return
    }
    exitCode = await run(args)
  } catch (error) {
    console.error(`fireline: ${(error as Error).message}`)
    exitCode = 1
  }
  process.exit(exitCode)
}

function parseArgs(argv: readonly string[]): ParsedArgs {
  const out = {
    command: 'run' as 'run' | 'help',
    file: null as string | null,
    port: 4440,
    streamsPort: 7474,
    stateStream: null as string | null,
    name: null as string | null,
    repl: false,
    providerOverride: null as string | null,
  }
  let i = 0
  if (argv[0] === 'run') i++
  if (argv[0] === '--help' || argv[0] === '-h') return { ...out, command: 'help' }

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
      case '--repl':
        out.repl = true
        break
      default:
        if (arg?.startsWith('--')) throw new Error(`unknown flag: ${arg}`)
        if (out.file) throw new Error(`unexpected argument: ${arg}`)
        out.file = arg
    }
  }
  return out
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

interface LoadedSpec {
  readonly sandbox: { readonly provider?: unknown }
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

function printReady(
  handle: { readonly id: string; readonly acp: { readonly url: string }; readonly state: { readonly url: string } },
  args: ParsedArgs,
): void {
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
