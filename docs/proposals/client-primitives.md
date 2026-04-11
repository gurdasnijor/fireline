# Proposal: Fireline Client Primitives (v2)

> **Status:** proposal, ready to execute
> **Supersedes:** [`../explorations/typescript-typed-functional-core-api.md`](../explorations/typescript-typed-functional-core-api.md) (v1, retained as the reasoning record)
> **Type:** design doc
> **Audience:** whoever is shipping the public `@fireline/client` surface and building the browser-harness demo on top of it
> **Source of truth:** [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md) — primitive anchoring, the seven-combinator decomposition, and the acceptance bars
> **Related:**
> - [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md) — the Anthropic-primitive table and the combinator algebra
> - [`./runtime-host-split.md`](./runtime-host-split.md) — the parallel Rust-side runtime refactor (parts of it become redundant once this proposal's `Host` primitive lands)
> - [`../explorations/typescript-typed-functional-core-api.md`](../explorations/typescript-typed-functional-core-api.md) — the v1 design this supersedes
> - `packages/state/src/collections/*` — the eight tanstack-react-db live collections that already exist and satisfy the Session read surface

## Purpose

This doc proposes the public TypeScript substrate API for Fireline, after a design conversation that surfaced three insights v1 got wrong:

1. **There is no client-side Session interface.** Clients read materialized state through `@fireline/state`'s existing tanstack-react-db collections. There is no `emitEvent` verb on the client side; the write path is ACP (prompt into a running runtime) or control-plane (wake/stop) or direct durable-streams producer (external appends like `approval_resolved`).
2. **Topology is not a primitive — combinators are.** The `managed-agents-mapping.md` doc documents seven base combinators (observe, mapEffect, appendToSession, filter, substitute, suspend, fanout) into which every existing Fireline component decomposes. A `Topology` is just a list of `Combinator` values. Named components (`approvalGate`, `budget`, `audit`) become pure helper functions that produce `Combinator` values, not first-class runtime types.
3. **`Host` and `Sandbox` are distinct primitives.** Anthropic's Sandbox primitive is the **tool-execution** environment inside a running session. What Fireline has been calling "the thing that provisions a runtime" is really the Host primitive — the thing that owns session lifecycle and exposes the `wake(session_id)` verb. Conflating them was a v1 mistake that made the Claude-managed satisfier (stress-tested below) impossible to express cleanly.

The rest of this doc describes the revised primitive surface, the module layout, concrete TypeScript signatures, a worked example against the Fireline host, a stress-test example against the Claude Agent SDK v2 preview's session-resume, and a build/migration order.

## What changed from v1

| v1 Module | v2 Disposition |
|---|---|
| `@fireline/client/core` | **Kept and narrowed.** Pure serializable types only: `Combinator` union + named helpers, `ResourceRef`, `ToolDescriptor`, `CapabilityRef`, `TransportRef`, `CredentialRef`, `SessionSpec`. |
| `@fireline/client/control` | **Deleted as a public module.** Control-plane interaction is an implementation detail of the Fireline `Host` satisfier, not a primitive. |
| `@fireline/client/acp` | **Kept as internal glue.** `connectAcp` stays but is an implementation concern of the Fireline `Host` — not the public primitive surface consumers compose against. |
| `@fireline/client/state` | **Deleted.** Already exists as the separate `@fireline/state` package with eight tanstack-react-db live collections. Consumers use that package directly. |
| `@fireline/client/orchestration` with staged `readResumeContext` → `ensureRuntimeForSession` → `tryLoadSession` → `resumeSession` | **Collapsed into a single `wake` primitive.** The staged breakdown was useful internally but is Fireline-specific. A general orchestrator only knows how to call `host.wake(handle)` and retry on failure. |
| `Session` interface with `getEvents` / `emitEvent` / `getPendingEvents` | **Deleted.** `@fireline/state` collections already cover the read side; there is no client-side event emission verb. |
| `Harness` as a public interface | **Deleted.** Harness is a runtime-internal concept. The combinator algebra (Topology) is the public substrate primitive; the harness loop that interprets combinators is a runtime concern. |
| `Sandbox` interface with `provision({resources}) + execute(name,input)` | **Renamed.** What v1 called Sandbox is really the `Host` primitive (session lifecycle). A separate, narrower `Sandbox` primitive exists for tool execution inside a running session — matching Anthropic's framing. |

## Module map

```
@fireline/client/
├── core/           pure serializable data — Combinator union, named helpers,
│                   ResourceRef, ToolDescriptor, CapabilityRef, SessionSpec
├── host/           Host primitive: createSession / wake / status / stopSession
├── orchestration/  Orchestrator primitive: wake-centric scheduler builders
├── sandbox/        Sandbox primitive (tool execution, separate from Host)
├── host-fireline/  Host satisfier that wraps the Fireline control plane + ACP
└── (optional) host-claude/  Host satisfier that wraps the Claude Agent SDK v2

@fireline/state/    UNCHANGED — existing package, eight tanstack-react-db
                    live collections backed by @durable-streams/state. This IS
                    the Session read surface. Clients import from here, not
                    from @fireline/client.
```

Splitting the Fireline-specific satisfier out (`host-fireline`) keeps `@fireline/client/core`, `host`, `orchestration`, and `sandbox` as pure-interface modules. Users who don't run a Fireline control plane at all can still depend on `@fireline/client` for the primitive types and interfaces, then bring their own host satisfier.

## Design goals (reinforced)

1. **Primitive-first.** Every public type maps to one of the Anthropic managed-agent primitives from `managed-agents-mapping.md` or it does not belong in the substrate.
2. **Data-first at the wire boundary.** Topologies, resources, capabilities, and session specs are durable, serializable values.
3. **Pure functions for local interpretation.** Combinator helpers, materializers, middleware — anything that executes in the TS process — stays functional.
4. **Open for local composition, closed for remote execution.** Any caller can compose `Combinator` values freely. But the runtime only interprets the seven documented combinator kinds. Custom behavior has two honest paths: add a new named combinator kind + runtime interpreter, or keep custom logic local in ACP middleware.
5. **Host satisfiers are independent of the primitive layer.** A new `Host` satisfier (Claude, microsandbox, Docker, Inngest-scheduled) never changes `@fireline/client/core` or `@fireline/client/host`.
6. **`@fireline/state` is the universal read layer.** Any host that mirrors its output into a durable stream using the existing STATE-PROTOCOL shape is observable by the same tanstack-react-db collections. The browser harness and any downstream UI is host-agnostic.

## Non-goals

- Exposing arbitrary closures that the runtime must execute. The combinator algebra is closed over known kinds; anything else is local middleware.
- Defining a product layer (`Run`, `Workspace`, `Profile`, `ApprovalQueue`). Those live above the substrate.
- Hiding the distinction between "runtime is alive" and "downstream agent has accepted the session" behind an opaque `resume()`. The `Host` primitive's `wake` verb is explicit about being idempotent and retry-safe; higher-level products can layer stronger guarantees if they want.
- A purely functional API for ACP. ACP is stateful by nature (WebSocket connection, session lifetime, streaming updates). The `Host` primitive abstracts over it, but the internal glue stays stateful.

---

## Module 1: `@fireline/client/core` — combinators, specs, refs

This module contains pure serializable data types and the helper functions that construct them. Zero runtime dependencies. Zero side effects.

### Shared building blocks

```ts
export type JsonValue =
  | null
  | boolean
  | number
  | string
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue }

export type JsonSchema = { readonly [key: string]: JsonValue }

export type Endpoint = {
  readonly url: string
  readonly headers?: Readonly<Record<string, string>>
}
```

### Combinators — the primitive substrate algebra

Every existing Fireline component decomposes into one of seven combinator kinds, per `managed-agents-mapping.md:76–101`. The TS types carry these combinators as data specs — no closures, no opaque functions — so the runtime can interpret them from the wire.

```ts
// Effect-pattern matchers — all the ways a combinator can select the
// effects it applies to. Every variant is data.
export type EffectPattern =
  | { readonly kind: 'any' }
  | { readonly kind: 'prompt_contains'; readonly needle: string }
  | { readonly kind: 'prompt_matches'; readonly regex: string; readonly flags?: string }
  | { readonly kind: 'tool_call'; readonly name?: string; readonly name_prefix?: string }
  | { readonly kind: 'peer_call' }
  | { readonly kind: 'any_of'; readonly patterns: readonly EffectPattern[] }

// Rewrite specs — what a combinator does to an effect when it matches.
// Again, data. The runtime dispatches on `kind`.
export type RewriteSpec =
  | { readonly kind: 'prepend_context'; readonly sources: readonly ContextSourceRef[] }
  | { readonly kind: 'route_to_peer'; readonly peer: string }
  | { readonly kind: 'replace_tool'; readonly from: string; readonly to: CapabilityRef }
  | { readonly kind: 'text_substitute'; readonly from: string; readonly to: string }

// Projection specs — how a combinator writes an effect to the Session log.
export type ProjectSpec =
  | { readonly kind: 'audit_effect' }
  | { readonly kind: 'durable_trace' }
  | { readonly kind: 'custom'; readonly entity_type: string }

// Suspension specs — why and when a suspend combinator pauses.
export type SuspendReasonSpec =
  | {
      readonly kind: 'require_approval'
      readonly scope: 'tool_calls' | 'all' | 'matching'
      readonly matcher?: EffectPattern    // only when scope === 'matching'
      readonly timeout_ms?: number
    }
  | { readonly kind: 'require_budget_refresh' }
  | { readonly kind: 'wait_for_peer'; readonly peer: string }

// External sinks for observe combinators — data refs, not callbacks.
export type ObserveSinkRef =
  | { readonly kind: 'state_stream'; readonly entity_type: string }
  | { readonly kind: 'metrics'; readonly name: string }

// Fanout / merge specs (parallelism primitive).
export type FanoutSplitSpec = { readonly kind: 'by_peer_list'; readonly peers: readonly string[] }
export type FanoutMergeSpec = { readonly kind: 'first_success' } | { readonly kind: 'all' }

// The combinator union itself. Every member is fully serializable.
export type Combinator =
  | { readonly kind: 'observe';           readonly sink: ObserveSinkRef }
  | { readonly kind: 'map_effect';        readonly rewrite: RewriteSpec;  readonly when?: EffectPattern }
  | { readonly kind: 'append_to_session'; readonly project: ProjectSpec;  readonly when?: EffectPattern }
  | { readonly kind: 'filter';            readonly when: EffectPattern;   readonly reject: JsonValue }
  | { readonly kind: 'substitute';        readonly rewrite: RewriteSpec;  readonly when: EffectPattern }
  | { readonly kind: 'suspend';           readonly reason: SuspendReasonSpec }
  | { readonly kind: 'fanout';            readonly split: FanoutSplitSpec; readonly merge: FanoutMergeSpec }

// A Topology is just an ordered list of combinators — no wrapper object.
export type Topology = readonly Combinator[]

// Composition is array construction. That's it.
export const topology = (...parts: readonly Combinator[]): Topology => parts

// Validation remains useful for diffing and preflight, but it is a pure
// function over values, not a runtime concern.
export declare function validateTopology(t: Topology): ValidationResult<Topology>
```

### Named combinator helpers

Every existing Fireline component maps to a pure helper that constructs a `Combinator`. The old product-named classes (`ApprovalGateComponent`, `BudgetComponent`, `ContextInjectionComponent`) stop existing as public TS concepts — they become implementation names inside the runtime's combinator interpreter.

```ts
export const observe = (sink: ObserveSinkRef): Combinator =>
  ({ kind: 'observe', sink })

// audit = an append_to_session projector that writes {kind: 'audit', effect}
export const audit = (): Combinator =>
  ({ kind: 'append_to_session', project: { kind: 'audit_effect' } })

// durableTrace = append_to_session with a bidirectional projector
export const durableTrace = (): Combinator =>
  ({ kind: 'append_to_session', project: { kind: 'durable_trace' } })

export const contextInjection = (sources: readonly ContextSourceRef[]): Combinator =>
  ({ kind: 'map_effect', rewrite: { kind: 'prepend_context', sources } })

// budget = filter that rejects when max_tokens exceeded. The "budget.check"
// logic lives inside the runtime's filter interpreter; the spec only names
// the knob.
export const budget = (tokens: number): Combinator =>
  ({ kind: 'filter', when: { kind: 'any' }, reject: { error: 'budget_exceeded', max_tokens: tokens } })

export const approvalGate = (opts: {
  readonly scope: 'tool_calls' | 'all'
  readonly timeoutMs?: number
}): Combinator => ({
  kind: 'suspend',
  reason: { kind: 'require_approval', scope: opts.scope, timeout_ms: opts.timeoutMs },
})

// Matcher-scoped approval — exactly the "pause_here" pattern the managed-
// agent harness test uses. Demo-friendly shorthand.
export const approvalGateOnPattern = (opts: {
  readonly matcher: EffectPattern
  readonly timeoutMs?: number
}): Combinator => ({
  kind: 'suspend',
  reason: { kind: 'require_approval', scope: 'matching', matcher: opts.matcher, timeout_ms: opts.timeoutMs },
})

// peer routing — substitute outgoing peer calls with a routed effect
export const peer = (peers: readonly string[]): Combinator =>
  ({ kind: 'substitute', rewrite: { kind: 'route_to_peer', peer: peers[0] /* simplification */ }, when: { kind: 'peer_call' } })

// parallel peer dispatch — fanout
export const parallelPeers = (peers: readonly string[]): Combinator =>
  ({ kind: 'fanout', split: { kind: 'by_peer_list', peers }, merge: { kind: 'first_success' } })
```

### Other core types (unchanged from v1, retained for completeness)

```ts
// Tools
export type ToolDescriptor = {
  readonly name: string
  readonly description: string
  readonly input_schema: JsonSchema
}

export type TransportRef =
  | { readonly kind: 'mcp_url'; readonly url: string }
  | { readonly kind: 'peer_runtime'; readonly runtime_key: string }
  | { readonly kind: 'smithery'; readonly catalog: string; readonly tool: string }
  | { readonly kind: 'in_process'; readonly component_name: string }

export type CredentialRef =
  | { readonly kind: 'env'; readonly var: string }
  | { readonly kind: 'secret'; readonly key: string }
  | { readonly kind: 'oauth_token'; readonly provider: string; readonly account?: string }

export type CapabilityRef = {
  readonly descriptor: ToolDescriptor
  readonly transport_ref: TransportRef
  readonly credential_ref?: CredentialRef
}

// Resources
export type ResourceRef =
  | { readonly kind: 'local_path'; readonly path: string; readonly mount_path: string; readonly read_only?: boolean }
  | { readonly kind: 'git_remote'; readonly repo_url: string; readonly ref?: string; readonly subdir?: string; readonly mount_path: string; readonly read_only?: boolean }
  | { readonly kind: 's3'; readonly bucket: string; readonly prefix: string; readonly mount_path: string }
  | { readonly kind: 'gcs'; readonly bucket: string; readonly prefix: string; readonly mount_path: string }

// Session spec — the "tell me what kind of session you want" payload.
// Notably a UNION of host-specific needs; each Host satisfier honors what
// it understands and ignores the rest.
export type SessionSpec = {
  readonly topology?: Topology                   // Fireline-host uses this
  readonly resources?: readonly ResourceRef[]    // Fireline-host uses this
  readonly capabilities?: readonly CapabilityRef[] // Fireline-host uses this (attach_tool)
  readonly agentCommand?: readonly string[]      // Fireline-host uses this
  readonly model?: string                        // Claude-host uses this
  readonly initialPrompt?: string                // optional first input for any host
  readonly metadata?: Readonly<Record<string, JsonValue>>
}
```

A future typing pass can split `SessionSpec` into discriminated-union variants per host kind. For v2 the union-of-fields shape is fine because each satisfier ignores fields it doesn't understand.

---

## Module 2: `@fireline/client/host` — the Host primitive

A `Host` is the thing that runs agent **sessions**. It owns session lifecycle and exposes the `wake` verb. This is the primitive the v1 proposal mistakenly called `Sandbox`.

```ts
// An opaque handle to a session the host has created. Host satisfiers
// define the shape. At minimum it carries an identifier the orchestrator
// can pass around.
export type SessionHandle = { readonly id: string; readonly kind: string }

// Status shape — hosts fill in their own state enum via `kind`.
export type SessionStatus =
  | { readonly kind: 'created' }
  | { readonly kind: 'running' }
  | { readonly kind: 'idle' }
  | { readonly kind: 'needs_wake' }
  | { readonly kind: 'stopped' }
  | { readonly kind: 'error'; readonly message: string }

// What wake returned. Orchestrators use this to decide whether to keep
// pumping or back off.
export type WakeOutcome =
  | { readonly kind: 'noop' }          // nothing to do; session is up to date
  | { readonly kind: 'advanced'; readonly steps: number }
  | { readonly kind: 'blocked'; readonly reason: SuspendReasonSpec }

// Optional streaming input surface. Not every host satisfies this — some
// are purely wake-driven with inputs persisted to a durable event registry.
export type SessionInput =
  | { readonly kind: 'prompt'; readonly text: string }
  | { readonly kind: 'tool_result'; readonly tool_call_id: string; readonly result: JsonValue }

export type SessionOutput =
  | { readonly kind: 'message'; readonly message: JsonValue }
  | { readonly kind: 'chunk'; readonly chunk: JsonValue }
  | { readonly kind: 'tool_call'; readonly tool_call: JsonValue }
  | { readonly kind: 'done' }

export interface Host {
  createSession(spec: SessionSpec): Promise<SessionHandle>
  wake(handle: SessionHandle): Promise<WakeOutcome>
  status(handle: SessionHandle): Promise<SessionStatus>
  stopSession(handle: SessionHandle): Promise<void>

  // Optional — hosts that support live streaming input (Fireline ACP,
  // Claude query streaming) implement this. Pure wake-driven hosts do not.
  sendInput?(handle: SessionHandle, input: SessionInput): AsyncIterable<SessionOutput>
}
```

### Host contract

- **`createSession`** reserves (or provisions) a session identifier. For Fireline this POSTs to the control plane and waits for `Ready`. For Claude-managed this is a local UUID plus an initial record in our durable stream.
- **`wake(handle)`** is the heart of the contract. It MUST be idempotent and retry-safe: calling it multiple times with the same handle — even concurrently — must converge on the same session state. It advances the session by one "step" (whatever that means for the host) and returns a `WakeOutcome`.
- **`status(handle)`** is observational. It never mutates. Orchestrators use it to decide whether to call `wake` again.
- **`stopSession(handle)`** tears down the session's execution state but does NOT delete the durable log. Subsequent calls to `createSession` with the same session ID (if the host supports it) should be able to resume from the log.
- **`sendInput`** is optional. Hosts that use it expose live streaming — user types into an input box, sees output stream back. Hosts without it require inputs to be written to the durable stream via external append, and then `wake` is called to drain them.

### The built-in `whileLoopOrchestrator` and the orchestrator contract

```ts
// @fireline/client/orchestration

export type WakeHandler = (session_id: string) => Promise<WakeOutcome>

export interface Orchestrator {
  // Queue a wake for a specific session. Retry-safe — multiple concurrent
  // calls for the same session_id coalesce.
  wakeOne(session_id: string): Promise<void>
  // Begin the scheduling loop. Different satisfiers use different schedulers:
  // while-loop, cron, queue consumer, HTTP endpoint.
  start(): Promise<void>
  stop(): Promise<void>
}

// The default satisfier — a while-loop that polls a session registry for
// sessions needing wake and calls a WakeHandler with retry.
export function whileLoopOrchestrator(opts: {
  readonly handler: WakeHandler
  readonly registry: SessionRegistry
  readonly pollIntervalMs?: number
  readonly maxConcurrent?: number
  readonly onError?: (err: unknown, session_id: string) => Promise<'retry' | 'drop'>
}): Orchestrator

// Alternate satisfiers — all conform to the same interface.
export function cronOrchestrator(opts: {
  readonly schedule: string        // cron expression
  readonly handler: WakeHandler
  readonly enumerate: () => Promise<readonly string[]>  // which session IDs to wake per tick
}): Orchestrator

export function httpOrchestrator(opts: {
  readonly handler: WakeHandler
  readonly listen: { readonly port: number; readonly path?: string }
}): Orchestrator
```

**The orchestrator is Host-independent.** It only knows how to call the `WakeHandler` with a session ID and retry. It does NOT know about `SessionHandle` at all — only strings. The handler's body is what dispatches to a `Host.wake(handle)`, and constructing that handle from the string is a satisfier-layer concern, not a primitive-layer one.

**Deliberately NOT included at the primitive layer:** a generic `orchestratorFor(host, opts)` helper or any `resolveHandle(host, session_id)` function. An earlier draft of this doc sketched such a helper; it was a design mistake. There is no generic way to reconstruct a `SessionHandle` from a `session_id` string without knowing the specific Host satisfier's handle shape, and if you know the satisfier you're already outside the primitive layer. Each Host satisfier provides its own convenience factory that closes over the host and produces a `WakeHandler` internally. For example:

```ts
// Tier 3 / satisfier-layer sketch — NOT part of @fireline/client/orchestration
// lives in @fireline/client/host-fireline instead
export function createFirelineHostOrchestrator(opts: FirelineHostOptions): Orchestrator {
  const host = createFirelineHost(opts)
  const handler: WakeHandler = async (session_id) => {
    // Fireline's session_id is the handle.id trivially
    await host.wake({ id: session_id, kind: 'fireline' })
  }
  return whileLoopOrchestrator({ handler, registry: defaultFirelineRegistry(opts), pollIntervalMs: 500 })
}
```

The Claude-host satisfier has an analogous factory that closes over its own dependencies and produces a `{ id: session_id, kind: 'claude' }` handle inside the closure. Neither satisfier's factory appears in the primitive-layer `@fireline/client/orchestration` module — they live in the respective host satisfier packages.

### `SessionRegistry` — reading "which sessions need wake" from `@fireline/state`

The registry abstraction decouples the orchestrator from HOW it finds sessions to wake. For Fireline, the registry queries `@fireline/state`'s session collections for rows with `state === 'needs_wake'` or with pending inputs. For Claude-managed, the registry queries the same durable stream for pending-input rows the Claude host satisfier wrote.

```ts
export interface SessionRegistry {
  listPending(): AsyncIterable<string>       // stream of session IDs needing wake
  onPendingChange(fn: () => void): Unsubscribe  // live notify when new pending appears
}

// Default satisfier — backed by @fireline/state collections.
export function streamSessionRegistry(opts: {
  readonly stateStreamUrl: string
  readonly filter?: (row: JsonValue) => boolean
}): SessionRegistry
```

The registry and orchestrator together give: "whenever a session-pending row hits the durable stream, call `host.wake` for that session ID with retry."

## Module 3: `@fireline/client/sandbox` — tool execution

This is the **Anthropic Sandbox primitive** from the managed-agent table: "any executor that can be configured once and called many times as a tool." It's the inside-the-session code-execution environment — bash, python, browser, microsandbox, etc.

```ts
// @fireline/client/sandbox

export type SandboxHandle = { readonly id: string; readonly kind: string }

export interface Sandbox {
  provision(opts: { readonly resources: readonly ResourceRef[] }): Promise<SandboxHandle>
  execute(
    handle: SandboxHandle,
    call: { readonly name: string; readonly input: JsonValue },
  ): Promise<{ readonly output: JsonValue; readonly exit_status?: number }>
  release(handle: SandboxHandle): Promise<void>
}
```

This is a SEPARATE primitive from `Host`. A `Host` can delegate tool execution to a `Sandbox` (either a bundled one like the Claude managed sandbox, or a user-provided one). Fireline-host's current topology gives it access to `fs_backend`, `peer_mcp`, and `attach_tool` components that collectively satisfy the Sandbox primitive; for now those stay internal and the public `Sandbox` interface is for future integration (e.g., the microsandbox evaluation — see `../handoff-2026-04-11-stream-as-truth-and-runtime-abstraction.md`).

Sandbox is documented here for completeness but is explicitly optional for v2. Demo-critical work can skip it.

## Module 4: `@fireline/state` (existing package) — the Session read surface

No new code in this module. Clients import directly from `@fireline/state`, which already exports eight tanstack-react-db live collections backed by `@durable-streams/state`:

- `sessionTurns` — per-session prompt-turn rows
- `turnChunks` — streaming chunks per turn
- `activeTurns` — turns currently in flight
- `queuedTurns` — turns queued behind a suspend
- `pendingPermissions` — in-flight permission_request rows (the approval gate read surface)
- `sessionPermissions` — full history of permission events per session
- `connectionTurns` — turn-by-connection joins
- — plus any future additions per `packages/state/src/schema.ts`

Clients use these with `useLiveQuery` / `useCollection` from `@tanstack/db`:

```ts
import { useLiveQuery } from '@tanstack/db'
import { pendingPermissions, sessionTurns } from '@fireline/state'

function ApprovalPanel({ sessionId }: { sessionId: string }) {
  const pending = useLiveQuery(pendingPermissions({ sessionId }))
  return (
    <ul>
      {pending.map(row => (
        <li key={row.requestId}>
          {row.reason}
          <button onClick={() => approve(row)}>Approve</button>
          <button onClick={() => deny(row)}>Deny</button>
        </li>
      ))}
    </ul>
  )
}
```

**There is no `@fireline/client/state` module. There is no `Session` interface.** The materialized state layer is already shipped and already reactive; the proposal does not invent a second way to read it.

### External writes that are not host writes

Some demo-relevant writes to the durable stream are neither `Host.wake` nor `Host.sendInput` — specifically, external approval responses. These are appends to the durable stream from a component outside the runtime (the browser UI, an admin CLI, a Slack bot).

The substrate pattern for these is already in place: `tests/support/managed_agent_suite.rs::append_approval_resolved` writes a `permission` entity_type row directly to the shared stream via `@durable-streams`'s client, and the runtime's approval-gate combinator interpreter is listening. A matching TS helper belongs in a thin utility module:

```ts
// @fireline/client/events — minimal helpers for external stream writes.
// NOT a primitive. These are demo / product conveniences.

export async function appendApprovalResolved(opts: {
  readonly stream: Endpoint
  readonly session_id: string
  readonly request_id: string
  readonly allow: boolean
  readonly resolved_by?: string
}): Promise<void>

export async function appendQueuedTurn(opts: {
  readonly stream: Endpoint
  readonly session_id: string
  readonly prompt: string
}): Promise<void>
```

These are thin wrappers over `@durable-streams` producers. They do not deserve a dedicated module and can live in whichever package feels right (likely `@fireline/client/host-fireline` or a tiny `@fireline/client/events` helper).

---

## Module 5: `@fireline/client/host-fireline` — the Fireline satisfier

This is where the existing Fireline control plane becomes one concrete `Host` satisfier. It wraps:

- `POST /v1/runtimes` → `createSession`
- `fireline::orchestration::resume` → `wake`
- `GET /v1/runtimes/{key}` → `status`
- `POST /v1/runtimes/{key}/stop` → `stopSession`
- ACP over WebSocket → `sendInput` (for live-streaming interactions)

```ts
// @fireline/client/host-fireline
import type { Host } from '@fireline/client/host'

export function createFirelineHost(opts: {
  readonly controlPlaneUrl: string
  readonly controlPlaneToken?: string
  readonly sharedStateUrl: string
}): Host

// Convenience bundle — returns the host plus a pre-wired orchestrator
// using @fireline/state as the session registry.
export function createFirelineClient(opts: {
  readonly controlPlaneUrl: string
  readonly controlPlaneToken?: string
  readonly sharedStateUrl: string
}): { readonly host: Host; readonly orchestrator: Orchestrator }
```

The existing `createHostClient` and `HostClient.resume(sessionId)` (from commit `4eaf94a`/`36096b7`) stay as backward-compatible wrappers. Their bodies become:

```ts
// Backward-compat shim
export function createHostClient(opts: HostClientOptions): HostClient {
  const host = createFirelineHost({ /* ... */ })
  return {
    create: (spec) => host.createSession(toSessionSpec(spec)),
    stop: (key) => host.stopSession({ id: key, kind: 'fireline' }),
    resume: (id) => host.wake({ id, kind: 'fireline' }),   // delegates to wake
    // ... rest of the HostClient surface, each method delegates
  }
}
```

No consumer-visible break. `HostClient.resume` keeps working; under the hood it's `host.wake`.

### Rust wire translator (for the Combinator → legacy TopologySpec bridge)

A small Rust adapter in `crates/fireline-conductor` translates each `Combinator` kind from the wire into the existing `TopologyComponentSpec { name, config }` shape that the legacy factory registry understands. This lets the TS API ship the clean combinator shape immediately without rewriting the Rust runtime's component factories.

```rust
// crates/fireline-conductor/src/topology_translator.rs (new, ~100-200 lines)

pub fn combinators_to_topology_spec(
    combinators: &[Combinator],
) -> TopologySpec {
    let components = combinators.iter().map(|c| match c {
        Combinator::Suspend { reason: SuspendReasonSpec::RequireApproval { scope, matcher, timeout_ms } } => {
            TopologyComponentSpec {
                name: "approval_gate".into(),
                config: Some(serde_json::json!({
                    "policies": [/* translate matcher */],
                    "timeoutMs": timeout_ms,
                })),
            }
        }
        Combinator::Filter { .. } => TopologyComponentSpec {
            name: "budget".into(),
            config: /* ... */,
        },
        // ... one arm per combinator kind
    }).collect();
    TopologySpec { components }
}
```

The translator is a one-way adapter for the demo-day path. A post-demo slice migrates the Rust runtime to interpret `Combinator` directly (collapsing the named factories into one combinator interpreter), which is cleaner but not critical.

---

## Worked example — Fireline host

```ts
import {
  topology,
  durableTrace,
  approvalGateOnPattern,
  type Combinator,
} from '@fireline/client/core'
import { createFirelineClient } from '@fireline/client/host-fireline'
import { useLiveQuery } from '@tanstack/db'
import { sessionTurns, turnChunks, pendingPermissions } from '@fireline/state'
import { appendApprovalResolved } from '@fireline/client/events'

// 1. Build a topology as pure data
const demoTopology: readonly Combinator[] = topology(
  durableTrace(),
  approvalGateOnPattern({
    matcher: { kind: 'prompt_contains', needle: 'pause_here' },
    timeoutMs: 15000,
  }),
)

// 2. Wire up the Fireline host + orchestrator
const { host, orchestrator } = createFirelineClient({
  controlPlaneUrl: 'http://127.0.0.1:4440',
  sharedStateUrl: 'http://127.0.0.1:4440/v1/stream/shared-state',
})
await orchestrator.start()

// 3. Create a session and send an initial prompt
const handle = await host.createSession({
  topology: demoTopology,
  agentCommand: ['fireline-testy-load'],
  initialPrompt: 'please pause_here for approval',
})

// 4. Stream the session interaction
if (host.sendInput) {
  for await (const output of host.sendInput(handle, { kind: 'prompt', text: '...' })) {
    // output.kind === 'chunk', 'message', 'tool_call', or 'done'
  }
}

// 5. In the demo UI, a separate component renders live state
function DemoApp() {
  const turns = useLiveQuery(sessionTurns({ sessionId: handle.id }))
  const chunks = useLiveQuery(turnChunks({ sessionId: handle.id }))
  const pending = useLiveQuery(pendingPermissions({ sessionId: handle.id }))

  return (
    <>
      <TurnsList turns={turns} />
      <ChunkStream chunks={chunks} />
      {pending.map(row => (
        <ApprovalRow
          key={row.requestId}
          row={row}
          onApprove={() => appendApprovalResolved({
            stream: { url: 'http://127.0.0.1:4440/v1/stream/shared-state' },
            session_id: row.sessionId,
            request_id: row.requestId,
            allow: true,
          })}
        />
      ))}
      <button onClick={() => host.stopSession(handle)}>Stop runtime</button>
      <button onClick={() => orchestrator.wakeOne(handle.id)}>Wake</button>
    </>
  )
}
```

Every import is a shipping primitive or a `@fireline/state` collection. There is no demo-only glue.

---

## Stress-test example — Claude Agent SDK v2 host

This satisfier proves the primitive interfaces are not Fireline-specific. It talks to the **Claude Agent SDK v2 preview** (`@anthropic-ai/claude-agent-sdk`'s `unstable_v2_*` surface) directly. No control plane, no runtime provisioning, no shared stream infrastructure from us — just a process-lifetime `Map<handle.id, SDKSession>` bridging our durable stream to Claude's live session object.

> **SDK reference:** the V2 preview surface is documented at [`https://code.claude.com/docs/en/agent-sdk/typescript-v2-preview`](https://code.claude.com/docs/en/agent-sdk/typescript-v2-preview). The full divergence analysis, V1-vs-V2 comparison, and the reasoning behind the shape below lives in [`../explorations/claude-agent-sdk-v2-findings.md`](../explorations/claude-agent-sdk-v2-findings.md). Anyone touching this section should read the findings doc first — the V2 programming model is meaningfully different from V1, and an earlier draft of this sketch was written against V1.

```ts
// @fireline/client/host-claude
import {
  unstable_v2_createSession,
  unstable_v2_resumeSession,
  type SDKMessage,
  type SDKSession,
} from '@anthropic-ai/claude-agent-sdk'
import type { Host, WakeOutcome } from '@fireline/client/host'

// Authentication: the SDK picks up ANTHROPIC_API_KEY from the environment.
// No explicit field on the options type — same convention as every other
// Anthropic TS SDK. If the SDK ever grows an explicit auth option, add it
// here; until then, env-var is the contract.

export function createClaudeHost(opts: {
  readonly model?: string
  readonly stateProducer: StreamProducer        // mirrors Claude output into our durable stream
  readonly pendingInputs: PendingInputRegistry  // user's way of saying "process this next"
}): Host {
  // Process-lifetime cache of live SDKSession handles. Rebuilt lazily
  // on wake() via unstable_v2_resumeSession when a handle is missing
  // (e.g. after a process restart). This is symmetric to how FirelineHost
  // transparently reconnects to a live runtime via RuntimeRegistry after
  // a control-plane restart.
  const live = new Map<string, SDKSession>()
  const model = opts.model ?? 'claude-opus-4-6'

  // Central acquire() helper — the bridge between "in-memory live session"
  // and "persistent session id in our durable stream". Handles both the
  // already-warm path and the cold-restart path. Robust to either close()
  // semantic: if close() turns out to destroy server-side state, the
  // resumeSession call will fail and the fallback path creates fresh.
  async function acquire(handleId: string): Promise<SDKSession> {
    const existing = live.get(handleId)
    if (existing) return existing

    // Restore from the durable stream — we stashed the Claude
    // sessionId on the first successful wake.
    const stashed = await opts.stateProducer.readOne({ type: 'session', key: handleId })
    const claudeSessionId: string | undefined = stashed?.value?.claudeSessionId
    try {
      const sdkSession = claudeSessionId
        ? unstable_v2_resumeSession(claudeSessionId, { model })
        : unstable_v2_createSession({ model })
      live.set(handleId, sdkSession)
      return sdkSession
    } catch (err) {
      // resumeSession may fail if the server-side state was dropped
      // (TTL expiry, deployment restart, close() actually deleted).
      // Fall back to a fresh session and log the divergence for the
      // durable trail.
      const sdkSession = unstable_v2_createSession({ model })
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
      // V2 session creation is synchronous — no round-trip required,
      // so we can do it eagerly. In V1 this was deferred to wake()
      // because query() was stateless-per-call.
      const sdkSession = unstable_v2_createSession({ model: spec.model ?? model })
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
          // V2 exposes sessionId directly on the SDKSession object —
          // no need to wait for an init system-message like V1 required.
          claudeSessionId: sdkSession.sessionId,
        },
      })
      if (spec.initialPrompt) {
        await opts.pendingInputs.push(id, { kind: 'prompt', text: spec.initialPrompt })
      }
      return { id, kind: 'claude' }
    },

    async wake(handle): Promise<WakeOutcome> {
      // 1. Drain pending inputs from our own stream (the user's way of
      //    saying "please advance this session with this next prompt").
      //    If none, return noop — no work to do.
      const pending = await opts.pendingInputs.drain(handle.id)
      if (pending.length === 0) return { kind: 'noop' }

      // 2. Acquire a live SDKSession for this handle — either from the
      //    in-memory cache or by resuming from the claudeSessionId we
      //    stashed on first createSession.
      const sdkSession = await acquire(handle.id)
      const prompt = pending.map(p => p.text).join('\n\n')

      // 3. V2 splits send() and stream() — send() dispatches the user
      //    message, stream() yields the agent response for THAT turn.
      //    This is different from V1's single query() call returning
      //    an async iterable.
      await sdkSession.send(prompt)

      // 4. Mirror everything the session emits into our durable stream.
      //    The @fireline/state collections see these rows reactively.
      //    Every SDKMessage carries session_id, so we can key by it.
      let steps = 0
      for await (const msg of sdkSession.stream()) {
        await opts.stateProducer.append({
          type: 'claude_message',
          key: `${handle.id}:${msg.session_id}:${steps}`,
          headers: { operation: 'insert' },
          value: msg,
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
      // V2 exposes session.close() for local cleanup. The server-side
      // semantics (whether close() destroys persisted session state or
      // just disconnects the local handle) are unverified from the V2
      // docs alone — see claude-agent-sdk-v2-findings.md §5. The
      // acquire() helper above is defensive against either outcome:
      // a subsequent wake() that tries resumeSession will fall back
      // to createSession if the server-side state is gone.
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
```

**~150 lines.** Plug it into the same `whileLoopOrchestrator` + `@fireline/state` collections the Fireline host uses. The browser demo UI is identical — it reads the same materialized collections, it calls `host.wake(handle)` and `host.createSession(spec)` and `host.stopSession(handle)`, it doesn't know or care whether the host is Fireline or Claude.

**What this proves about the design — and why the V2 divergence actually validates it:**

The V2 programming model is fundamentally different from V1. V1 had a single stateless `query({ prompt, options: { resume } })` call that returned an async iterable. V2 has three entrypoints (`unstable_v2_createSession`, `unstable_v2_resumeSession`, `unstable_v2_prompt`) and a live `SDKSession` object with a `send()` / `stream()` split that you hold across turns. **Despite the drastic shape change, the `Host` primitive interface survives unchanged.** That is the critical finding from the stress test.

Specifically:

1. **`Host` is primitive-shaped, not SDK-shaped.** Both "spawn a Rust subprocess via HTTP control plane + WebSocket ACP" (FirelineHost) and "hold an SDKSession with per-turn send/stream" (ClaudeHost V2) satisfy the same four-method interface — `createSession / wake / status / stopSession`. The interface abstracts over the satisfier's internal state model entirely. Had we stayed with the earlier `Sandbox.provision + execute + shutdown` conflation, the V2 divergence would've been much worse: V2's `SDKSession` is inherently session-lifetime, not per-tool-execution, and the old conflated shape would've forced awkward bridging code. **The Host/Sandbox cleave is what makes both V1 and V2 satisfiers trivial.**

2. **`wake(handle)` is the right universal verb.** It admits every known programming model: V1's one-shot `query()` call, V2's per-turn `send/stream`, Fireline's control-plane reprovision + ACP session/load, a cron job calling `wake` over HTTP. The verb only requires "idempotent, retry-safe advance by one step" and every satisfier can choose its own internal model.

3. **`@fireline/state` is the universal read surface.** Every host mirrors its output via the STATE-PROTOCOL shape into the durable stream, and the same tanstack-react-db collections render in the browser regardless of which host produced the rows. Host-agnostic UI.

4. **The `Combinator` + `Topology` types are Fireline-specific by design.** The Claude host ignores them — V2's topology is opaque to us, controlled by Anthropic's managed service. That's fine. `SessionSpec` is a union-of-needs per the earlier design decision; each host honors the fields it understands.

5. **Session lifecycle state can live in the satisfier, not the interface.** FirelineHost keeps runtime_keys in the control plane's RuntimeRegistry. ClaudeHost keeps `SDKSession` handles in a process-lifetime `Map`. Neither leaks into the Host interface. The `acquire(handleId)` pattern in the V2 sketch above is the pure-functional-ish equivalent of the Fireline satisfier's lazy runtime reconnection — both satisfy the same contract ("give me back a ready session regardless of process-restart state"), both invisible through the Host trait.

**Unresolved before a concrete ClaudeHost ships:** how V2 surfaces `tool_use` / `tool_result` events in the `stream()` generator. The V2 docs describe the session's assistant message flow but not the tool-execution model. If V2 streams `tool_use` events back to the caller and expects `tool_result` to be sent via `send()`, then ClaudeHost can delegate tool execution to a separate `Sandbox` satisfier (e.g. `MicrosandboxSandbox`) — which is the §7 story in `runtime-host-split.md` working as intended. If V2 bundles tool execution inside Claude's managed sandbox opaquely, the Sandbox-delegation story doesn't apply to ClaudeHost and the scope shrinks. This is the one genuine blocker for Step 3 (Tier E) code; see `claude-agent-sdk-v2-findings.md` §5 for the open items list.

**Open question:** The wake loop assumes each wake processes a pending prompt. If the user wants a cron that calls `wake` on a timer without a pending input, the `noop` return is correct. Future work: a `wake(..., reason: 'poll')` variant for polling-style satisfiers. Out of scope for v2; file as a follow-up if it comes up.

---

## Build / migration order

Each tier is individually reviewable and individually useful.

### Tier 1 — `@fireline/client/core` (half day)

- `Combinator` union + supporting spec types (`EffectPattern`, `RewriteSpec`, `ProjectSpec`, `SuspendReasonSpec`, `ObserveSinkRef`, `FanoutSplitSpec`, `FanoutMergeSpec`)
- Named helpers: `observe`, `audit`, `durableTrace`, `contextInjection`, `budget`, `approvalGate`, `approvalGateOnPattern`, `peer`, `parallelPeers`
- `topology(...)` factory, `validateTopology`
- `ResourceRef`, `ToolDescriptor`, `CapabilityRef`, `TransportRef`, `CredentialRef`, `SessionSpec`
- Zero runtime dependencies. All pure. Unit tests verify JSON round-trips.

### Tier 2 — `@fireline/client/host` + `@fireline/client/orchestration` (day)

- `Host` interface + `SessionHandle`, `SessionStatus`, `WakeOutcome`, `SessionInput`, `SessionOutput` types
- `Orchestrator` interface + `WakeHandler = (session_id: string) => Promise<void>` type. Note: `WakeHandler` deals in strings, never in `SessionHandle`. Handle construction is a satisfier-layer concern.
- `whileLoopOrchestrator` live builder + `cronOrchestrator` / `httpOrchestrator` stubbed signatures (future satisfiers)
- `SessionRegistry` interface (`listPending(): AsyncIterable<string>`, `onPendingChange`) + `streamSessionRegistry` default satisfier
- **No generic `orchestratorFor(host)` convenience factory.** See the "Deliberately NOT included at the primitive layer" callout in Module 2 — a generic `orchestratorFor` requires resolving a `SessionHandle` from a string without knowing the satisfier's handle shape, which isn't possible at the interface layer. Per-satisfier convenience factories live in Tier 3 (`createFirelineHostOrchestrator`) and Tier 6 (`createClaudeHostOrchestrator`) respectively.
- Zero runtime dependencies on Fireline specifics. Tests use a mock `Host`.

### Tier 3 — `@fireline/client/host-fireline` (day)

- `createFirelineHost` satisfier wrapping the existing control plane + ACP
- `createFirelineClient` convenience bundle (host + orchestrator pre-wired)
- Backward-compat shim: `createHostClient` / `HostClient.resume` delegate to `Host.wake`
- The existing `packages/client/src/host.ts` → refactored to export the new surface alongside the legacy one
- Existing `packages/client/test/host.test.ts` → extended to cover the `Host`-interface path

### Tier 4 — Rust combinator → legacy TopologySpec translator (half day)

- `crates/fireline-conductor/src/topology_translator.rs` — pure function `fn translate(&[Combinator]) -> TopologySpec`
- Wired into the control plane's `create_runtime` handler so incoming `POST /v1/runtimes` bodies can carry a `combinators` field in addition to (or instead of) the legacy `topology.components` field
- Zero changes to the existing Rust component factories — they stay named (`approval_gate`, `budget`, `context_injection`) and keep their current configs
- Unit tests assert one combinator kind → one legacy component for each of the seven combinator shapes

### Tier 5 — Demo rewrite (half day)

- `packages/browser-harness/src/app.tsx` re-plumbed to use `@fireline/client/core` + `@fireline/client/host-fireline` + `@fireline/state` collections
- Add an `ApprovalPanel` reading `useLiveQuery(pendingPermissions({ sessionId }))`
- Add a "Stop runtime" and "Wake" button pair that exercises `Host.stopSession` + `Orchestrator.wakeOne`
- The existing state inspector UI stays — it already reads from tanstack-react-db collections

### Tier 6 — (optional) `@fireline/client/host-claude` (day)

- `createClaudeHost` satisfier wrapping `@anthropic-ai/agent-sdk`'s `query({ resume })`
- Shipped as a separate sub-package so users who don't want the Claude dependency don't pull it in
- Demo rewrite gets a toggle: "run against Fireline host" / "run against Claude host". Same UI.

### Tier 7 — (post-demo) Rust combinator interpreter collapse

- Delete the per-component factories in the Rust runtime (`ApprovalGateComponent`, `BudgetComponent`, `ContextInjectionComponent`, etc.) as distinct types
- Replace with a single `CombinatorInterpreter` that dispatches on the `kind` field of each combinator in the spec
- This makes the seven-combinator framing load-bearing instead of documentation
- Likely surfaces hidden coupling (the combinators aren't perfectly independent today)
- Not on the critical path; cleanup after the primitive surface is shipped

**Total critical path (Tiers 1–5): ~3 days of focused work.** Optional Tier 6 adds a day for the Claude stress-test satisfier. Tier 7 is weeks and can wait.

---

## Open questions

1. **Claude SDK version.** The v2 preview's exact `query` signature may differ from the sketch above. The stress-test example needs verification against the actual SDK docs at `code.claude.com/docs/en/agent-sdk/typescript-v2-preview#session-resume` before landing `@fireline/client/host-claude`. The shape is believed correct; field names may not be.
2. **`SessionSpec` discriminated union vs. union-of-fields.** v2 ships the union-of-fields shape (each host ignores fields it doesn't understand). Future typing pass could split into `FirelineSessionSpec | ClaudeSessionSpec | DockerSessionSpec`. Decide when a third or fourth satisfier exists to ground the typing.
3. **Package layout.** Should `host-fireline` and `host-claude` ship as separate sub-packages (`@fireline/client-fireline`, `@fireline/client-claude`) or as internal modules of `@fireline/client` that are tree-shakeable? The former makes optional deps cleaner but adds workspace-management overhead. Lean toward sub-packages for the Claude case (it has a real optional dep on `@anthropic-ai/agent-sdk`); Fireline can stay inline.
4. **`sendInput` vs. `wake`-only.** Some hosts (Fireline ACP) naturally expose streaming input. Others (Claude managed via wake-only) don't. Demo UI needs streaming for the chat display, which argues for keeping `sendInput` as a first-class optional Host method. But the combinator interpreter in the Fireline case is driven by wake (not sendInput), so the two paths overlap. A future pass might unify them. For v2, keep `sendInput` as optional and document which hosts implement it.
5. **Tool execution via the separate `Sandbox` primitive.** v2 proposes the interface but doesn't require shipping a satisfier on the critical path. The microsandbox evaluation (see `../handoff-2026-04-11-stream-as-truth-and-runtime-abstraction.md`) is the natural forcing function for the first real `Sandbox` satisfier.
6. **Event-write helpers in `@fireline/client/events`.** Do external writes like `appendApprovalResolved` deserve a dedicated tiny module, or do they live inside `host-fireline`? Argues-for-dedicated: they work against any durable-streams host, not just Fireline. Argues-for-inline: demo-driven work only exercises them from the browser-harness which already imports `host-fireline`. Lean toward a dedicated tiny module so Claude-host users can write external approval events without depending on `host-fireline`.
7. **What `status` should expose.** The rough `SessionStatus` kinds (`created`/`running`/`idle`/`needs_wake`/`stopped`/`error`) are adequate for a demo but poorer than Fireline's full `RuntimeStatus` union (which has `Busy`/`Stale` etc.). Either (a) include an opaque `details: JsonValue` field for host-specific state or (b) let hosts add variants. Probably (a) — variants don't compose across hosts.

---

## Supporting types referenced

Types referenced above but not fully expanded here. Each is straightforward and belongs in `@fireline/client/core`:

- `ContextSourceRef` — `{ kind: 'static_text' | 'workspace_file' | 'datetime', ... }`
- `ValidationResult<T>` — `{ ok: true; value: T } | { ok: false; errors: readonly ValidationError[] }`
- `Unsubscribe` — `() => void`
- `StreamProducer` — thin wrapper over `@durable-streams` client's producer, `append(envelope) → Promise<void>`
- `PendingInputRegistry` — Claude-host's pending-input store, `push / drain / peek`

---

## Appendix: what v2 preserves from v1

This is a significant redraft of v1 but it preserves v1's load-bearing design commitments:

- **Data-first at the wire boundary.** Combinator values are serializable, diffable, testable. Closures never cross the TS/Rust boundary.
- **Composability through pure functions over values.** Named helpers produce `Combinator`, `topology(...)` produces `Topology`. No builder state.
- **Primitive-first.** Every public type maps to an Anthropic managed-agent primitive. The combinator algebra is the concrete realization of that commitment.
- **Verifiability over cleverness.** `validateTopology`, JSON round-trip tests, and the `@fireline/client/core` module staying zero-runtime-dep are all in service of the same goal v1 named.
- **No product objects.** No `Run`, `Workspace`, `Profile`, or `ApprovalQueue`. The browser harness can build those as downstream abstractions on top of the primitive reads.

What v2 changes is the shape — three layers of simplification (no Session interface, no heavyweight topology wrapper, Host/Sandbox split) — not the design philosophy.

## Closing

The design conversation that produced v2 stress-tested the primitive interfaces against two real satisfiers (Fireline control plane, Claude Agent SDK v2) and one skeptical product layer (browser harness reading materialized state). The interfaces held up in both stress tests, and the simplifications that fell out are each genuine: **the seven combinators were already in the mapping doc, `@fireline/state` already has the live collections, and the `Host` vs. `Sandbox` split was always implicit in the Anthropic primitive table.** v1 had the right philosophy but the wrong shape. v2 is what the philosophy produces when you take each insight seriously.

Execute in the tier order above. Stop after Tier 5 for demo-day. Resume with Tier 6 (Claude host) as the stress-test demonstration if time permits, and Tier 7 (Rust combinator interpreter collapse) as post-demo cleanup.
