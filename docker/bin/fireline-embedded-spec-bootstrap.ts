import { readFile } from 'node:fs/promises'
import { spawn } from 'node:child_process'

import { compose } from '../../packages/client/src/sandbox.ts'

type JsonRecord = Record<string, unknown>

type EmbeddedHarnessSpec = {
  readonly kind: 'harness'
  readonly name?: string
  readonly stateStream?: string
  readonly sandbox: JsonRecord
  readonly middleware: JsonRecord
  readonly agent: JsonRecord
}

type LoweredProvisionRequest = {
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

const SPEC_ENV = 'FIRELINE_EMBEDDED_SPEC_PATH'
const FIRELINE_BIN = '/usr/local/bin/fireline'

async function main(): Promise<void> {
  const specPath = process.env[SPEC_ENV]
  if (!specPath) {
    throw new Error(`${SPEC_ENV} must be set`)
  }

  const spec = parseSpec(await readFile(specPath, 'utf8'))
  const lowered = await lowerSpec(spec)
  validateLoweredSpec(lowered)

  const args = buildDirectHostArgs(lowered)
  console.log(
    `fireline: booting embedded spec '${lowered.name}' from ${specPath} via existing compose()->start lowering`,
  )

  const child = spawn(FIRELINE_BIN, args, {
    stdio: 'inherit',
    env: process.env,
  })

  const forwardSignal = (signal: NodeJS.Signals) => {
    if (!child.killed) child.kill(signal)
  }
  process.on('SIGINT', () => forwardSignal('SIGINT'))
  process.on('SIGTERM', () => forwardSignal('SIGTERM'))

  child.on('exit', (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal)
      return
    }
    process.exit(code ?? 1)
  })
}

function parseSpec(raw: string): EmbeddedHarnessSpec {
  const parsed = JSON.parse(raw) as Partial<EmbeddedHarnessSpec>
  if (parsed.kind !== 'harness') {
    throw new Error(`embedded spec must be kind='harness'; got ${String(parsed.kind)}`)
  }
  if (!parsed.sandbox || !parsed.middleware || !parsed.agent) {
    throw new Error('embedded spec must include sandbox, middleware, and agent sections')
  }
  return {
    kind: 'harness',
    name: parsed.name,
    stateStream: parsed.stateStream,
    sandbox: parsed.sandbox,
    middleware: parsed.middleware,
    agent: parsed.agent,
  }
}

async function lowerSpec(spec: EmbeddedHarnessSpec): Promise<LoweredProvisionRequest> {
  const originalFetch = globalThis.fetch
  let lowered: LoweredProvisionRequest | null = null

  globalThis.fetch = async (_input, init) => {
    if (init?.method !== 'POST' || typeof init.body !== 'string') {
      throw new Error('embedded-spec bootstrap expected compose()->start() to POST /v1/sandboxes')
    }
    lowered = JSON.parse(init.body) as LoweredProvisionRequest
    return new Response(
      JSON.stringify({
        id: 'embedded-spec-bootstrap',
        provider: 'local',
        acp: { url: 'ws://embedded-spec-bootstrap.invalid/acp' },
        state: { url: 'http://embedded-spec-bootstrap.invalid/state' },
      }),
      {
        status: 201,
        headers: { 'content-type': 'application/json' },
      },
    )
  }

  try {
    await compose(spec.sandbox as never, spec.middleware as never, spec.agent as never).start({
      serverUrl: 'http://embedded-spec-bootstrap.invalid',
      name: spec.name,
      stateStream: spec.stateStream,
    })
  } finally {
    globalThis.fetch = originalFetch
  }

  if (!lowered) {
    throw new Error('compose()->start() lowering did not emit a provision request')
  }
  return lowered
}

function validateLoweredSpec(spec: LoweredProvisionRequest): void {
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
  if ((spec.resources?.length ?? 0) > 0) {
    throw new Error('embedded-spec boot does not support resource mounts')
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

function buildDirectHostArgs(spec: LoweredProvisionRequest): string[] {
  const host = process.env.FIRELINE_HOST ?? '0.0.0.0'
  const port = process.env.FIRELINE_PORT ?? '4440'
  const durableStreamsUrl = process.env.FIRELINE_DURABLE_STREAMS_URL
  if (!durableStreamsUrl) {
    throw new Error('FIRELINE_DURABLE_STREAMS_URL must be set for embedded-spec boot')
  }
  const advertisedStateStreamUrl = process.env.FIRELINE_ADVERTISED_STATE_STREAM_URL
    ?? (spec.stateStream ? `${durableStreamsUrl.replace(/\/+$/, '')}/${spec.stateStream}` : null)

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

  args.push('--', ...spec.agentCommand)
  return args
}

function hasTopologyComponents(topology: unknown): boolean {
  if (!topology || typeof topology !== 'object') {
    return false
  }
  const components = (topology as { readonly components?: unknown }).components
  return Array.isArray(components) && components.length > 0
}

main().catch((error) => {
  console.error(`fireline: embedded-spec bootstrap failed: ${(error as Error).message}`)
  process.exit(1)
})
