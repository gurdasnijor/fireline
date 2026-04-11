/**
 * Claude Agent SDK v2 Host satisfier — Tier 4 of the client-primitives plan
 * described in `docs/proposals/client-primitives.md`
 * ("Stress-test example — Claude Agent SDK v2 host", §6 Appendix) and the
 * V2 surface verified in
 * `docs/explorations/claude-agent-sdk-v2-findings.md`.
 *
 * The satisfier is deliberately symmetric to
 * {@link import('../host-fireline/client.js').createFirelineHost}: a file
 * shape of `client.ts + index.ts`, a small options interface, and no
 * behaviour beyond what the `Host` primitive contract requires. It proves
 * that the `Host` / `wake()` shape accommodates a fundamentally different
 * programming model (a live `SDKSession` with per-turn `send()`/`stream()`)
 * with no interface changes.
 *
 * ## Usage (doc-comment example)
 *
 * ```ts
 * import { createClaudeHost } from '@fireline/client/host-claude'
 *
 * const host = createClaudeHost({
 *   model: 'claude-opus-4-6',
 *   stateProducer, // your DurableStateProducer (append / readOne)
 *   pendingInputs, // your PendingInputRegistry (push / drain / peek)
 * })
 *
 * const handle = await host.createSession({ initialPrompt: 'hello' })
 * const outcome = await host.wake(handle)
 * //   => { kind: 'advanced', steps: N }
 * ```
 *
 * Every import is type-only because `@anthropic-ai/claude-agent-sdk` is a
 * peer dependency: the package does not need to be installed for
 * `pnpm -C packages/client lint` to pass. A co-located ambient stub in
 * `./claude-agent-sdk.d.ts` supplies the V2 surface declarations.
 */
import type {
  SDKMessage,
  SDKSession,
} from '@anthropic-ai/claude-agent-sdk'

import type { JsonValue } from '../core/index.js'
import type { Host, WakeOutcome } from '../host/index.js'

/**
 * Minimal envelope shape the satisfier reads and writes against the
 * durable state stream. Kept local to avoid coupling to any particular
 * durable-streams client version; a real adapter is trivially a thin
 * wrapper around `@durable-streams/client`'s producer.
 */
export interface StateEnvelope {
  readonly type: string
  readonly key: string
  readonly headers?: Readonly<Record<string, JsonValue>>
  readonly value: JsonValue
}

/**
 * The state row {@link createClaudeHost} reads back on `acquire()`. This
 * is what `readOne({ type: 'session', key: handleId })` is expected to
 * return when a session already exists in the durable stream.
 */
export interface StashedSessionRow {
  readonly value?: {
    readonly claudeSessionId?: string
    readonly [key: string]: JsonValue | undefined
  }
}

/**
 * The durable-stream surface the satisfier depends on. This is
 * intentionally named `DurableStateProducer` to match the Tier 4 brief;
 * it is a subset of what `@durable-streams/client`'s producer exposes in
 * production, narrowed to the two operations `acquire()` / `wake()` /
 * `createSession()` / `stopSession()` actually invoke.
 *
 * Keeping the shape narrow preserves the stream-as-truth principle from
 * `docs/handoff-2026-04-11-stream-as-truth-and-runtime-abstraction.md`:
 * the satisfier never caches session existence in memory alone — every
 * `acquire()` on a process-restart miss consults the durable stream via
 * `readOne` before deciding between `resumeSession` and `createSession`.
 */
export interface DurableStateProducer {
  append(envelope: StateEnvelope): Promise<void>
  readOne(query: {
    readonly type: string
    readonly key: string
  }): Promise<StashedSessionRow | null>
}

/**
 * Pending-input registry. User-facing code pushes prompts here; the
 * satisfier drains them on `wake()` to advance the live `SDKSession` by
 * one turn.
 */
export interface PendingInputRegistry {
  push(sessionId: string, input: PendingInput): Promise<void>
  drain(sessionId: string): Promise<readonly PendingInput[]>
  peek(sessionId: string): Promise<readonly PendingInput[]>
}

/**
 * The only pending-input kind the Claude satisfier needs today. Kept as
 * a discriminated union so tool-result routing can be added later (see
 * TODO(tier4-followup) in `stopSession` / `wake`).
 */
export type PendingInput = { readonly kind: 'prompt'; readonly text: string }

/**
 * Options accepted by {@link createClaudeHost}.
 *
 * `apiKey` is present but unused: the V2 docs do not enumerate an explicit
 * auth field on `createSession`'s options type, and the Anthropic TS SDK
 * convention is to resolve `ANTHROPIC_API_KEY` from the environment. We
 * accept it here to keep the call site stable in case the V2 options type
 * grows an explicit field; if / when that happens, plumb it through to
 * `unstable_v2_createSession` / `unstable_v2_resumeSession` below. See
 * `docs/explorations/claude-agent-sdk-v2-findings.md` §5 open item #2.
 */
export interface ClaudeHostOptions {
  readonly model: string
  readonly stateProducer: DurableStateProducer
  readonly pendingInputs: PendingInputRegistry
  readonly apiKey?: string
}

const DEFAULT_MODEL_FALLBACK = 'claude-opus-4-6'

export function createClaudeHost(opts: ClaudeHostOptions): Host {
  // Process-lifetime cache of live SDKSession handles. Rebuilt lazily on
  // wake() via unstable_v2_resumeSession when a handle is missing (e.g.
  // after a process restart). Symmetric to how FirelineHost transparently
  // reconnects to a live runtime via RuntimeRegistry after a control-plane
  // restart — see `docs/proposals/client-primitives.md` §"Stress-test
  // example — Claude Agent SDK v2 host", point (5).
  const live = new Map<string, SDKSession>()
  const model = opts.model ?? DEFAULT_MODEL_FALLBACK

  // Lazy dynamic import of the SDK. Because the package is a peer dep we
  // never statically reference a runtime symbol — only types — so the
  // `import()` expression is the single point where a missing install
  // will surface. This also lets the satisfier be constructed (for tests
  // / wiring checks) even on machines where the SDK isn't installed.
  async function loadSdk(): Promise<{
    readonly unstable_v2_createSession: (options: { model: string }) => SDKSession
    readonly unstable_v2_resumeSession: (
      sessionId: string,
      options: { model: string },
    ) => SDKSession
  }> {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const mod: any = await import(
      /* @vite-ignore */ '@anthropic-ai/claude-agent-sdk'
    )
    return {
      unstable_v2_createSession: mod.unstable_v2_createSession,
      unstable_v2_resumeSession: mod.unstable_v2_resumeSession,
    }
  }

  // Central acquire() helper — the bridge between "in-memory live
  // session" and "persistent session id in our durable stream". On cache
  // miss it ALWAYS consults the durable stream via `readOne` first; the
  // in-memory map is a convenience, never a source of truth.
  async function acquire(handleId: string): Promise<SDKSession> {
    const existing = live.get(handleId)
    if (existing) return existing

    const sdk = await loadSdk()

    // Restore from the durable stream — we stashed the Claude sessionId
    // on the first successful createSession / wake.
    const stashed = await opts.stateProducer.readOne({
      type: 'session',
      key: handleId,
    })
    const claudeSessionId: string | undefined = stashed?.value?.claudeSessionId

    try {
      const sdkSession = claudeSessionId
        ? sdk.unstable_v2_resumeSession(claudeSessionId, { model })
        : sdk.unstable_v2_createSession({ model })
      live.set(handleId, sdkSession)
      return sdkSession
    } catch (err) {
      // resumeSession may fail if the server-side state was dropped (TTL
      // expiry, deployment restart, close() actually deleted). Fall back
      // to a fresh session and log the divergence for the durable trail.
      const sdkSession = sdk.unstable_v2_createSession({ model })
      live.set(handleId, sdkSession)
      await opts.stateProducer.append({
        type: 'session',
        key: handleId,
        headers: { operation: 'update' },
        value: {
          claudeSessionId: sdkSession.sessionId,
          state: 'running',
          note: `resume failed, recreated fresh: ${String(err)}`,
        },
      })
      return sdkSession
    }
  }

  return {
    async createSession(spec) {
      const id = `claude:${crypto.randomUUID()}`
      // V2 session creation is synchronous — no round-trip required, so
      // we can do it eagerly. In V1 this was deferred to wake() because
      // query() was stateless-per-call.
      const sdk = await loadSdk()
      const sdkSession = sdk.unstable_v2_createSession({ model: spec.model ?? model })
      live.set(id, sdkSession)
      await opts.stateProducer.append({
        type: 'session',
        key: id,
        headers: { operation: 'insert' },
        value: {
          sessionId: id,
          host: 'claude',
          model: spec.model ?? model,
          state: 'created',
          createdAt: Date.now(),
          // V2 exposes sessionId directly on the SDKSession object — no
          // need to wait for an init system-message like V1 required.
          claudeSessionId: sdkSession.sessionId,
        },
      })
      if (spec.initialPrompt) {
        await opts.pendingInputs.push(id, { kind: 'prompt', text: spec.initialPrompt })
      }
      return { id, kind: 'claude' }
    },

    async wake(handle): Promise<WakeOutcome> {
      // 1. Drain pending inputs from our own stream. If none, return
      //    noop — no work to do.
      const pending = await opts.pendingInputs.drain(handle.id)
      if (pending.length === 0) return { kind: 'noop' }

      // 2. Acquire a live SDKSession for this handle — either from the
      //    in-memory cache or by resuming from the claudeSessionId we
      //    stashed on first createSession. `acquire()` always consults
      //    the durable stream on miss; the Map is a convenience cache.
      const sdkSession = await acquire(handle.id)
      const prompt = pending.map((p) => p.text).join('\n\n')

      // 3. V2 splits send() and stream() — send() dispatches the user
      //    message, stream() yields the agent response for THAT turn.
      await sdkSession.send(prompt)

      // 4. Mirror everything the session emits into our durable stream.
      //    Every SDKMessage carries session_id, so we can key by it.
      //
      //    TODO(tier4-followup): V2's tool_use / tool_result handling
      //    model is unverified — see
      //    `docs/explorations/claude-agent-sdk-v2-findings.md` §5 open
      //    item #1. For now we mirror every message verbatim and let
      //    downstream readers interpret them.
      let steps = 0
      for await (const msg of sdkSession.stream()) {
        await opts.stateProducer.append({
          type: 'claude_message',
          key: `${handle.id}:${msg.session_id}:${steps}`,
          headers: { operation: 'insert' },
          value: msg as unknown as JsonValue,
        })
        steps += 1
      }

      // 5. Update the durable state row and mark pending as resolved.
      await opts.stateProducer.append({
        type: 'session',
        key: handle.id,
        headers: { operation: 'update' },
        value: { state: 'running', claudeSessionId: sdkSession.sessionId },
      })
      await opts.stateProducer.append({
        type: 'pending_resolved',
        key: handle.id,
        headers: { operation: 'insert' },
        value: { count: pending.length, resolvedAt: Date.now() },
      })

      return { kind: 'advanced', steps }
    },

    async status(handle) {
      const pending = await opts.pendingInputs.peek(handle.id)
      return pending.length > 0 ? { kind: 'needs_wake' } : { kind: 'idle' }
    },

    async stopSession(handle) {
      // TODO(tier4-followup): V2's `close()` semantics are unverified —
      // the docs do not clarify whether it destroys server-side session
      // state or only disconnects the local handle. See
      // `docs/explorations/claude-agent-sdk-v2-findings.md` §5 open
      // item #3. The acquire() helper above is defensive either way: a
      // subsequent wake() that tries resumeSession will fall back to
      // createSession if the server-side state is gone.
      const sdkSession = live.get(handle.id)
      sdkSession?.close()
      live.delete(handle.id)
      await opts.stateProducer.append({
        type: 'session',
        key: handle.id,
        headers: { operation: 'update' },
        value: { state: 'stopped' },
      })
    },
  }
}

// Re-export the SDK message type so downstream consumers can pattern-match
// on claude_message envelopes without taking a direct dependency on the
// (peer) SDK package.
export type { SDKMessage }
