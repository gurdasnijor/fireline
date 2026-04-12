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
3. **`Host` and `Sandbox` are distinct primitives, and `Host` hands out *runtimes*, not sessions.** Anthropic's Sandbox primitive is the **tool-execution** environment inside a running runtime. The Host primitive is the thing that provisions a runtime and exposes the `wake(handle)` verb. **Sessions live on the ACP data plane inside a provisioned runtime — not as a Host-primitive verb.** Conflating Sandbox and Host was a v1 mistake that made the Claude-managed stress test impossible to express cleanly, and conflating Host with session lifecycle was a semantic drift from Tier 2 that the `37db346` rename fixed (`createSession → provision`, `SessionHandle → HostHandle` carrying `acp` + `state` endpoints, `SessionSpec → ProvisionSpec`, `SessionStatus → HostStatus`, `stopSession → stop`, `sendInput` / `SessionInput` / `SessionOutput` deleted).

The rest of this doc describes the revised primitive surface, the module layout, concrete TypeScript signatures, a worked example against the Fireline host, a retrospective on the Claude Agent SDK v2 stress test (attempted and deleted in commit `37db346`), and a build/migration order.

## What changed from v1

| v1 Module | v2 Disposition |
|---|---|
| `@fireline/client/core` | **Kept and narrowed.** Pure serializable types only: `Combinator` union + named helpers, `ResourceRef`, `ToolDescriptor`, `CapabilityRef`, `TransportRef`, `CredentialRef`, `ProvisionSpec`. |
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
│                   ResourceRef, ToolDescriptor, CapabilityRef, ProvisionSpec
├── host/           Host primitive: provision / wake / status / stop
├── orchestration/  Orchestrator primitive: wake-centric scheduler builders
├── sandbox/        Sandbox primitive (tool execution, separate from Host)
└── host-fireline/  Host satisfier that wraps the Fireline control plane + ACP

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
export type ProvisionSpec = {
  readonly topology?: Topology                   // Fireline-host uses this
  readonly resources?: readonly ResourceRef[]    // Fireline-host uses this
  readonly capabilities?: readonly CapabilityRef[] // Fireline-host uses this (attach_tool)
  readonly agentCommand?: readonly string[]      // Fireline-host uses this
  readonly model?: string                        // Claude-host uses this
  readonly initialPrompt?: string                // optional first input for any host
  readonly metadata?: Readonly<Record<string, JsonValue>>
}
```

A future typing pass can split `ProvisionSpec` into discriminated-union variants per host kind. For v2 the union-of-fields shape is fine because each satisfier ignores fields it doesn't understand.

---

## Module 2: `@fireline/client/host` — the Host primitive

A `Host` is the thing that hands you a **runtime** — a place where an agent process can run. It owns runtime lifecycle and exposes the `wake` verb. Sessions live *inside* a provisioned runtime on the ACP data plane and are minted by `session/new`; the Host primitive does not own a session verb.

```ts
// An opaque handle to a runtime the host has provisioned. Satisfiers
// define the shape. At minimum it carries an identifier the orchestrator
// can pass around, plus the ACP and state-stream endpoints the caller
// needs to actually talk to the runtime — so downstream code doesn't
// have to hardcode a proxy URL.
export type HostHandle = {
  readonly id: string
  readonly kind: string
  readonly acp: Endpoint
  readonly state: Endpoint
}

// Status shape — hosts fill in their own state enum via `kind`.
export type HostStatus =
  | { readonly kind: 'created' }
  | { readonly kind: 'running' }
  | { readonly kind: 'idle' }
  | { readonly kind: 'needs_wake' }
  | { readonly kind: 'stopped' }
  | { readonly kind: 'error'; readonly message: string }

// What wake returned. Orchestrators use this to decide whether to keep
// pumping or back off.
export type WakeOutcome =
  | { readonly kind: 'noop' }          // nothing to do; runtime is up to date
  | { readonly kind: 'advanced'; readonly steps: number }
  | { readonly kind: 'blocked'; readonly reason: SuspendReasonSpec }

export interface Host {
  provision(spec: ProvisionSpec): Promise<HostHandle>
  wake(handle: HostHandle): Promise<WakeOutcome>
  status(handle: HostHandle): Promise<HostStatus>
  stop(handle: HostHandle): Promise<void>
}
```

Note what's deliberately **not** here: no `sendInput` method, no `SessionInput` / `SessionOutput` types, no session lifecycle verb. Live input into a running agent is an ACP data-plane concern — clients open their own WebSocket to `handle.acp.url` and speak ACP directly (via `@agentclientprotocol/sdk`'s `ClientSideConnection`). The Host primitive's job ends once the runtime is reachable; what happens over the ACP socket after that is the agent and the client's business, not the primitive's.

### Host contract

- **`provision`** hands out a runtime. For Fireline this POSTs `/v1/runtimes` on the control plane and waits for `Ready`. For a hypothetical hosted-API satisfier this allocates a runtime id and endpoint on the remote service. The returned `HostHandle` carries both the runtime identifier and the `acp` + `state` endpoints the caller needs to reach the runtime.
- **`wake(handle)`** is the heart of the contract. It MUST be idempotent and retry-safe: calling it multiple times with the same handle — even concurrently — must converge on the same runtime state. It advances the runtime by one "step" (whatever that means for the host) and returns a `WakeOutcome`. Per the TLA spec, `wake` on a `ready` runtime is a noop; `wake` on a `stopped` runtime reprovisions with a new `runtime_id` and preserves session bindings.
- **`status(handle)`** is observational. It never mutates. Orchestrators use it to decide whether to call `wake` again.
- **`stop(handle)`** tears down the runtime's execution state but does NOT delete the durable log. Subsequent calls to `provision` with an equivalent spec can re-bind the same logical runtime, and ACP sessions that were living inside survive on the durable stream for later replay via `session/load`.

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

**The orchestrator is Host-independent.** It only knows how to call the `WakeHandler` with a session ID and retry. It does NOT know about `HostHandle` at all — only strings. The handler's body is what dispatches to a `Host.wake(handle)`, and constructing that handle from the string is a satisfier-layer concern, not a primitive-layer one.

**Deliberately NOT included at the primitive layer:** a generic `orchestratorFor(host, opts)` helper or any `resolveHandle(host, session_id)` function. An earlier draft of this doc sketched such a helper; it was a design mistake. There is no generic way to reconstruct a `HostHandle` from a `session_id` string without knowing the specific Host satisfier's handle shape, and if you know the satisfier you're already outside the primitive layer. Each Host satisfier provides its own convenience factory that closes over the host and produces a `WakeHandler` internally. For example:

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

Some demo-relevant writes to the durable stream are neither `Host.wake` nor live ACP input — specifically, external approval responses. These are appends to the durable stream from a component outside the runtime (the browser UI, an admin CLI, a Slack bot).

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

- `POST /v1/runtimes` → `provision`
- `fireline::orchestration::resume` → `wake`
- `GET /v1/runtimes/{key}` → `status`
- `POST /v1/runtimes/{key}/stop` → `stop`
- ACP over WebSocket is **not** wrapped by the Host primitive — callers read `handle.acp.url` off the returned `HostHandle` and open an `@agentclientprotocol/sdk` `ClientSideConnection` directly

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
    create: (spec) => host.provision(toProvisionSpec(spec)),
    stop: (key) => host.stop({ id: key, kind: 'fireline', acp: { url: '' }, state: { url: '' } }),
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

// 3. Provision a runtime and record the initial prompt on the durable stream
const handle = await host.provision({
  topology: demoTopology,
  agentCommand: ['fireline-testy-load'],
  initialPrompt: 'please pause_here for approval',
})

// 4. Talk to the agent over ACP directly — the Host primitive doesn't
//    wrap the data plane. `handle.acp.url` is the WebSocket to open.
import { ClientSideConnection, PROTOCOL_VERSION } from '@agentclientprotocol/sdk'
const connection = new ClientSideConnection(/* ...handler... */, createWebSocket(handle.acp.url))
await connection.initialize({ protocolVersion: PROTOCOL_VERSION, clientCapabilities: { fs: { readTextFile: false } }, clientInfo: { name: 'demo', version: '0.0.1' } })
const { sessionId } = await connection.newSession({ cwd: '/', mcpServers: [] })
await connection.prompt({ sessionId, prompt: [{ type: 'text', text: 'please pause_here for approval' }] })

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
      <button onClick={() => host.stop(handle)}>Stop runtime</button>
      <button onClick={() => orchestrator.wakeOne(handle.id)}>Wake</button>
    </>
  )
}
```

Every import is a shipping primitive or a `@fireline/state` collection. There is no demo-only glue.

---

## Stress test, in retrospect — the Claude Agent SDK v2 thought experiment

**Status: the host-claude satisfier was attempted, informed the design, and was deleted in commit `37db346`.**

An earlier draft of this doc carried a ~150-line `createClaudeHost` sketch as the stress-test example: *"this satisfier proves the primitive interfaces are not Fireline-specific."* The full code walk, its divergence analysis against the V2 preview SDK, and the design conclusions it produced are preserved as design history in [`../explorations/claude-agent-sdk-v2-findings.md`](../explorations/claude-agent-sdk-v2-findings.md). Anyone reasoning about a future second-satisfier should start there.

The exercise taught three things that are now load-bearing in the design above:

1. **`Host` is primitive-shaped, not SDK-shaped.** Both the fireline-native satisfier and the Claude V2 SDK (two radically different internal programming models — control-plane + ACP WebSocket vs. live `SDKSession` with a `send/stream` split) fit the same four-method `Host` interface without bridging code. That validated the Host/Sandbox cleave: had we stayed with v1's conflated `Sandbox.provision + execute + shutdown` shape, V2's `SDKSession` object model would have forced awkward per-tool-execution glue.
2. **`wake(handle)` is the right universal verb.** It admits every programming model we could name: stateless-per-call, live-session-with-send/stream, control-plane-reprovision, cron over HTTP. Only "idempotent, retry-safe advance" is required; every satisfier picks its own internal state model.
3. **A `Host` should hand back a *runtime*, not a *session*.** The Claude V2 exercise surfaced a semantic drift that had been hiding in the `Host.createSession` name since Tier 2: the primitive's job is to hand you a place where an agent can run, and sessions live inside that place on the ACP data plane (or whatever equivalent the satisfier exposes). The rename landed in commit `37db346` — `createSession → provision`, `SessionHandle → HostHandle` (now carrying `acp` + `state` endpoints so callers don't hardcode a proxy URL), `SessionSpec → ProvisionSpec`, `SessionStatus → HostStatus`, `stopSession → stop`, `sendInput` deleted (clients speak ACP directly via `handle.acp.url`), `SessionInput` / `SessionOutput` deleted, and `packages/client/src/host-claude/` deleted.

The satisfier code itself was removed because (a) the V2 preview's tool-execution model was never clarified well enough to commit to a Sandbox-delegation story without guesswork, and (b) its genuine contribution was the design lessons above, which are now baked into the main body of this doc.

---

## Build / migration order

Each tier is individually reviewable and individually useful.

### Tier 1 — `@fireline/client/core` (half day)

- `Combinator` union + supporting spec types (`EffectPattern`, `RewriteSpec`, `ProjectSpec`, `SuspendReasonSpec`, `ObserveSinkRef`, `FanoutSplitSpec`, `FanoutMergeSpec`)
- Named helpers: `observe`, `audit`, `durableTrace`, `contextInjection`, `budget`, `approvalGate`, `approvalGateOnPattern`, `peer`, `parallelPeers`
- `topology(...)` factory, `validateTopology`
- `ResourceRef`, `ToolDescriptor`, `CapabilityRef`, `TransportRef`, `CredentialRef`, `ProvisionSpec`
- Zero runtime dependencies. All pure. Unit tests verify JSON round-trips.

### Tier 2 — `@fireline/client/host` + `@fireline/client/orchestration` (day)

- `Host` interface + `HostHandle`, `HostStatus`, `WakeOutcome`, `ProvisionSpec` types
- `Orchestrator` interface + `WakeHandler = (id: string) => Promise<void>` type. Note: `WakeHandler` deals in strings (a `HostHandle.id`), never in `HostHandle` objects. Handle construction is a satisfier-layer concern.
- `whileLoopOrchestrator` live builder + `cronOrchestrator` / `httpOrchestrator` stubbed signatures (future satisfiers)
- `SessionRegistry` interface (`listPending(): AsyncIterable<string>`, `onPendingChange`) + `streamSessionRegistry` default satisfier. (The name predates the `37db346` rename and still reflects the orchestrator's job — watching durable-stream rows that represent long-running logical work — even though at the Host primitive layer the id passed to `wake` is a `HostHandle.id` rather than an ACP session id. A follow-up pass may rename the registry type to `HandleRegistry`; not urgent.)
- **No generic `orchestratorFor(host)` convenience factory.** See the "Deliberately NOT included at the primitive layer" callout in Module 2 — a generic `orchestratorFor` requires resolving a `HostHandle` from a string without knowing the satisfier's handle shape, which isn't possible at the interface layer. Per-satisfier convenience factories live in Tier 3 (`createFirelineHostOrchestrator`).
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
- Add a "Stop runtime" and "Wake" button pair that exercises `Host.stop` + `Orchestrator.wakeOne`
- The existing state inspector UI stays — it already reads from tanstack-react-db collections

### ~~Tier 6 — (optional) `@fireline/client/host-claude`~~ (deleted)

The optional Claude-host satisfier tier was attempted, informed the `Host.provision` / `HostHandle` rename in commit `37db346`, and was then deleted from the tree along with its code in the same commit. The design conclusions are preserved in §6 "Stress test, in retrospect" above and in [`../explorations/claude-agent-sdk-v2-findings.md`](../explorations/claude-agent-sdk-v2-findings.md). If a second real `Host` satisfier lands in a future tier, it should start from the primitive surface as it stands today, not from the Claude V2 sketch.

### Tier 7 — (post-demo) Rust combinator interpreter collapse

- Delete the per-component factories in the Rust runtime (`ApprovalGateComponent`, `BudgetComponent`, `ContextInjectionComponent`, etc.) as distinct types
- Replace with a single `CombinatorInterpreter` that dispatches on the `kind` field of each combinator in the spec
- This makes the seven-combinator framing load-bearing instead of documentation
- Likely surfaces hidden coupling (the combinators aren't perfectly independent today)
- Not on the critical path; cleanup after the primitive surface is shipped

**Total critical path (Tiers 1–5): ~3 days of focused work.** Tier 6 (Claude stress-test satisfier) was attempted and deleted; its design takeaways are baked into the main body. Tier 7 is weeks and can wait.

---

## Open questions

1. **`ProvisionSpec` discriminated union vs. union-of-fields.** v2 ships the union-of-fields shape (each host ignores fields it doesn't understand). Future typing pass could split into `FirelineProvisionSpec | DockerProvisionSpec | RemoteApiProvisionSpec` etc. Decide when a third or fourth satisfier exists to ground the typing.
2. **Package layout for future satisfiers.** If/when a non-Fireline `Host` satisfier ships (e.g. a remote hosted-API satisfier), it should probably live as a separate sub-package (`@fireline/client-<satisfier>`) so users who don't want that dependency don't pull it in. The Fireline satisfier can stay inline in `@fireline/client/host-fireline`. The deleted `host-claude` attempt had a real optional dep on `@anthropic-ai/claude-agent-sdk`; that's what the sub-package pattern is for.
3. **Tool execution via the separate `Sandbox` primitive.** v2 proposes the interface but doesn't require shipping a satisfier on the critical path. The microsandbox evaluation (see `../handoff-2026-04-11-stream-as-truth-and-runtime-abstraction.md`) is the natural forcing function for the first real `Sandbox` satisfier.
4. **Event-write helpers in `@fireline/client/events`.** Do external writes like `appendApprovalResolved` deserve a dedicated tiny module, or do they live inside `host-fireline`? Argues-for-dedicated: they work against any durable-streams host, not just Fireline. Argues-for-inline: demo-driven work only exercises them from the browser-harness which already imports `host-fireline`. Lean toward a dedicated tiny module so non-fireline hosts can write external approval events without depending on `host-fireline`.
5. **What `status` should expose.** The rough `HostStatus` kinds (`created`/`running`/`idle`/`needs_wake`/`stopped`/`error`) are adequate for a demo but poorer than Fireline's full `RuntimeStatus` union (which has `Busy`/`Stale` etc.). Either (a) include an opaque `details: JsonValue` field for host-specific state or (b) let hosts add variants. Probably (a) — variants don't compose across hosts.

---

## Supporting types referenced

Types referenced above but not fully expanded here. Each is straightforward and belongs in `@fireline/client/core`:

- `ContextSourceRef` — `{ kind: 'static_text' | 'workspace_file' | 'datetime', ... }`
- `ValidationResult<T>` — `{ ok: true; value: T } | { ok: false; errors: readonly ValidationError[] }`
- `Unsubscribe` — `() => void` (lives in `@fireline/client/core` per the `37db346` reorg; was previously in a deleted `core/session.ts`)
- `Endpoint` — `{ readonly url: string; readonly headers?: Readonly<Record<string, string>> }`, used by `HostHandle.acp` and `HostHandle.state`
- `StreamProducer` — thin wrapper over `@durable-streams` client's producer, `append(envelope) → Promise<void>`

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

The design conversation that produced v2 stress-tested the primitive interfaces against two candidate satisfiers (Fireline control plane, Claude Agent SDK v2 — the latter attempted and deleted, with its design lessons preserved in §6) and one skeptical product layer (browser harness reading materialized state). The interfaces held up in both stress tests, and the simplifications that fell out are each genuine: **the seven combinators were already in the mapping doc, `@fireline/state` already has the live collections, and the `Host` vs. `Sandbox` split was always implicit in the Anthropic primitive table.** v1 had the right philosophy but the wrong shape. v2 is what the philosophy produces when you take each insight seriously — and the `provision` rename in `37db346` is what happens when you take the second-satisfier stress test seriously enough to notice the semantic drift in the first satisfier's method name.

Execute in the tier order above. Stop after Tier 5 for demo-day. Tier 7 (Rust combinator interpreter collapse) is post-demo cleanup.
