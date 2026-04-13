import { readFile } from 'node:fs/promises'
import { spawn } from 'node:child_process'

import { compose } from '../../packages/client/src/sandbox.ts'
import {
  buildDirectHostArgs,
  type LoweredProvisionRequest,
  validateLoweredSpec,
} from '../../packages/fireline/src/embedded-spec-bootstrap.ts'

type JsonRecord = Record<string, unknown>

type EmbeddedHarnessSpec = {
  readonly kind: 'harness'
  readonly name?: string
  readonly stateStream?: string
  readonly sandbox: JsonRecord
  readonly middleware: JsonRecord
  readonly agent: JsonRecord
}

const SPEC_ENV = 'FIRELINE_EMBEDDED_SPEC_PATH'
const FIRELINE_BIN = process.env.FIRELINE_BIN ?? '/usr/local/bin/fireline'

async function main(): Promise<void> {
  const specPath = process.env[SPEC_ENV]
  if (!specPath) {
    throw new Error(`${SPEC_ENV} must be set`)
  }

  const spec = parseSpec(await readFile(specPath, 'utf8'))
  const lowered = await lowerSpec(spec)
  validateLoweredSpec(lowered)

  const args = await buildDirectHostArgs(lowered)
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

main().catch((error) => {
  console.error(`fireline: embedded-spec bootstrap failed: ${(error as Error).message}`)
  process.exit(1)
})
