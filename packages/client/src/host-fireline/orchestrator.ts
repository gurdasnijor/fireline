/**
 * Fireline-specific orchestration conveniences that close over the Fireline
 * Host satisfier, as described in `docs/proposals/client-primitives.md` and
 * anchored to `docs/explorations/managed-agents-mapping.md`.
 */
import type { Host } from '../host/index.js'
import type { Orchestrator, WakeHandler } from '../orchestration/index.js'
import { whileLoopOrchestrator } from '../orchestration/index.js'
import { createFirelineHost, type FirelineHostOptions } from './client.js'
import { createStreamSessionRegistry } from './registry.js'

export interface FirelineHostOrchestratorOptions extends FirelineHostOptions {
  readonly pollIntervalMs?: number
}

export function createFirelineHostOrchestrator(
  opts: FirelineHostOrchestratorOptions,
): Orchestrator {
  const host = createFirelineHost(opts)
  return buildFirelineHostOrchestrator(host, opts)
}

export function createFirelineClient(opts: FirelineHostOrchestratorOptions): {
  readonly host: Host
  readonly orchestrator: Orchestrator
} {
  const host = createFirelineHost(opts)
  const orchestrator = buildFirelineHostOrchestrator(host, opts)
  return { host, orchestrator }
}

function buildFirelineHostOrchestrator(
  host: Host,
  opts: FirelineHostOrchestratorOptions,
): Orchestrator {
  const registry = createStreamSessionRegistry({ sharedStateUrl: opts.sharedStateUrl })
  const handler: WakeHandler = async (session_id) => {
    const outcome = await host.wake({
      id: session_id,
      kind: 'fireline',
      acp: { url: '' },
      state: { url: opts.sharedStateUrl },
    })
    if (outcome.kind === 'blocked') {
      throw new Error(`wake blocked on ${session_id}: ${JSON.stringify(outcome.reason)}`)
    }
  }

  return whileLoopOrchestrator({
    handler,
    registry,
    pollIntervalMs: opts.pollIntervalMs ?? 500,
  })
}
