# Fireline TypeScript API — Typed Functional Core

> **SUPERSEDED** by [`../proposals/client-api-redesign.md`](../proposals/client-api-redesign.md) and [`../proposals/sandbox-provider-model.md`](../proposals/sandbox-provider-model.md). Retained for architectural history.
> Status: historical exploration, superseded first by `client-primitives.md` and now by the client API redesign
> Type: design doc
> Audience: maintainers deciding the low-level TypeScript surface
> Source of truth at the time: [`./managed-agents-mapping.md`](./managed-agents-mapping.md)
> Related:
> - [`./managed-agents-mapping.md`](./managed-agents-mapping.md) — primitive anchoring and acceptance bars
> - [`./typescript-functional-api-proposal.md`](./typescript-functional-api-proposal.md) — earlier function-valued proposal
> - [`../ts/low-level-api-surface.md`](../ts/low-level-api-surface.md) — namespace-based proposal
> - [`../ts/primitives.md`](../ts/primitives.md) — current primitive inventory

## Purpose

This doc proposes a better TypeScript substrate API for Fireline: a **typed
functional core** built from:

- **serializable values** for anything the runtime must execute remotely
- **pure functions** for anything that executes locally in the TypeScript
  process
- **small interpreters** that bridge those values into the control plane, ACP,
  and durable state stream

The goal is to keep the API:

- primitive-anchored against [`managed-agents-mapping.md`](./managed-agents-mapping.md)
- easy to compose into a higher product layer
- easy to verify and test
- honest about current substrate guarantees

This is a middle path between the two existing proposals:

- better than the namespace-heavy API because composition is first-class
- better than the closure-valued functional proposal because the wire boundary
  stays explicit and serializable

## Core Position

The previous functional proposal got one important thing right:

> Fireline is composition over a small algebra, not a pile of product objects.

But it put the functional boundary in the wrong place by making runtime
topology a public higher-order function:

```ts
type Component = (next: Harness) => Harness
```

That is a good **internal implementation model**. It is not the right **public
TypeScript substrate model**, because public TS values must survive:

- serialization over the wire
- persistence in specs and state
- diffing and validation
- execution by a Rust runtime that cannot run arbitrary TS closures

The better rule is:

1. **If the runtime executes it, model it as data.**
2. **If the local TS process executes it, model it as a pure function.**

That gives Fireline two small algebras instead of one confused one:

- **Remote algebra**: serializable `TopologySpec`, `ResourceRef`,
  `CapabilityRef`, `RuntimeSpec`
- **Local algebra**: `Materializer<S>`, `AcpMiddleware`, selectors, and helper
  composition functions

This keeps the API functional without pretending remote compute is just a TS
closure.

## Design Goals

1. **Primitive-first.** Every public type maps to one or two primitives from
   [`managed-agents-mapping.md`](./managed-agents-mapping.md), or it does not
   belong in the substrate.
2. **Data-first at the wire boundary.** Topologies, resources, capabilities,
   and runtime specs are durable values, not executable JS.
3. **Pure functions for local interpretation.** Materializers, selectors, and
   ACP middleware stay functional because they execute locally.
4. **Hot/cold split remains visible.** ACP traffic, control-plane lifecycle,
   and durable state reads are separate seams.
5. **No product objects.** No `Run`, `Workspace`, `Profile`, or `ApprovalQueue`
   in the low-level API.
6. **Composability over builders.** Composition should happen through pure
   functions over values, not mutable builder state.
7. **Verifiability over cleverness.** The API should support validation,
   normalization, diffing, and exact assertions in tests.

## Non-Goals

- Exposing raw `mapEffect(fn)` or `substitute(fn)` closures as a public runtime
  extension API
- Letting arbitrary TS code run inside the runtime
- Hiding orchestration complexity behind a deceptively simple `resume()` that
  promises stronger semantics than the substrate provides today
- Collapsing control plane, ACP, and durable state into a single opaque client
  object

## Recommended Shape

The recommended public surface is a **layered functional API**:

```ts
// logical modules; actual package layout can be decided separately
@fireline/client/core
@fireline/client/control
@fireline/client/acp
@fireline/client/state
@fireline/client/orchestration
@fireline/client        // optional convenience re-exports
```

The important point is not the exact package split. The important point is that
the surface is separated by **interpreter boundary**:

- `core`: pure, serializable data constructors and validators
- `control`: free functions that interpret specs against the control plane
- `acp`: free functions that interpret ACP endpoints into live connections
- `state`: free functions that interpret stream endpoints into materialized
  views
- `orchestration`: composition helpers over `control + state + acp`

## Module 1: `core` — Serializable Functional Algebra

This is the real functional core.

Everything in `core` is:

- immutable
- serializable
- comparable
- testable without side effects

### Core Types

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

export type RuntimeDescriptor = {
  readonly runtime_key: string
  readonly runtime_id: string
  readonly node_id: string
  readonly provider: 'local' | 'docker' | string
  readonly provider_instance_id: string
  readonly status: 'starting' | 'ready' | 'busy' | 'idle' | 'stale' | 'broken' | 'stopped'
  readonly acp: Endpoint
  readonly state: Endpoint
  readonly helper_api_base_url?: string
  readonly created_at_ms: number
  readonly updated_at_ms: number
}
```

### Tools: Separate Descriptor from Portability Ref

This is a critical design choice.

The **Tools primitive** is the agent-visible schema triple:

```ts
export type ToolDescriptor = {
  readonly name: string
  readonly description: string
  readonly input_schema: JsonSchema
}
```

Portable launch-time wiring is separate:

```ts
export type TransportRef =
  | { readonly kind: 'mcp_url'; readonly url: string }
  | { readonly kind: 'peer_runtime'; readonly runtime_key: string }
  | { readonly kind: 'smithery'; readonly catalog: string; readonly tool: string }

export type CredentialRef =
  | { readonly kind: 'env'; readonly var: string }
  | { readonly kind: 'secret'; readonly key: string }
  | { readonly kind: 'oauth_token'; readonly provider: string; readonly account?: string }

export type CapabilityRef = {
  readonly descriptor: ToolDescriptor
  readonly transport_ref: TransportRef
  readonly credential_ref?: CredentialRef
}
```

Why this split is better:

- it keeps the Anthropic Tools primitive honest
- it lets Resources/Tools portability evolve without polluting the base tool
  descriptor
- it makes it obvious which parts are agent-visible and which are substrate
  plumbing

### Resources: Plain Launch-Time Refs

```ts
export type ResourceRef =
  | {
      readonly kind: 'local_path'
      readonly path: string
      readonly mount_path: string
      readonly read_only?: boolean
    }
  | {
      readonly kind: 'git_remote'
      readonly repo_url: string
      readonly mount_path: string
      readonly ref?: string
      readonly subdir?: string
      readonly read_only?: boolean
    }
```

### Topology: Data, Not Closures

The runtime-executed topology is a serializable list of component specs:

```ts
export type ComponentSpec =
  | { readonly kind: 'audit'; readonly metadata?: Readonly<Record<string, JsonValue>> }
  | { readonly kind: 'context_injection'; readonly sources: readonly ContextSourceRef[] }
  | { readonly kind: 'budget'; readonly tokens: number }
  | { readonly kind: 'approval_gate'; readonly scope: 'tool_calls' | 'all'; readonly timeout_ms?: number }
  | { readonly kind: 'peer'; readonly peers: readonly string[] }
  | { readonly kind: 'smithery'; readonly catalog: string; readonly tools: readonly string[] }
  | { readonly kind: 'fs_backend'; readonly backend: FileBackendRef }
  | { readonly kind: 'register_tool'; readonly tool: ToolDescriptor }
  | { readonly kind: 'attach_capability'; readonly capability: CapabilityRef }

export type TopologySpec = readonly ComponentSpec[]
```

This preserves the mapping doc's algebraic story without making the public API
lie about executability. Each built-in factory is still a small functional unit:

```ts
export const audit = (
  metadata?: Readonly<Record<string, JsonValue>>,
): ComponentSpec => ({ kind: 'audit', metadata })

export const contextInjection = (
  sources: readonly ContextSourceRef[],
): ComponentSpec => ({ kind: 'context_injection', sources })

export const budget = (tokens: number): ComponentSpec => ({
  kind: 'budget',
  tokens,
})

export const approvalGate = (opts: {
  readonly scope: 'tool_calls' | 'all'
  readonly timeout_ms?: number
}): ComponentSpec => ({ kind: 'approval_gate', ...opts })

export const registerTool = (tool: ToolDescriptor): ComponentSpec => ({
  kind: 'register_tool',
  tool,
})
```

### Topology Composition Helpers

Composition remains functional, but over values:

```ts
export type TopologyPart = ComponentSpec | TopologySpec

export const composeTopology = (...parts: readonly TopologyPart[]): TopologySpec =>
  parts.flatMap(part => Array.isArray(part) ? part : [part])

export const appendComponents = (
  topology: TopologySpec,
  ...components: readonly ComponentSpec[]
): TopologySpec => [...topology, ...components]

export const prependComponents = (
  topology: TopologySpec,
  ...components: readonly ComponentSpec[]
): TopologySpec => [...components, ...topology]

export const validateTopology = (topology: TopologySpec): ValidationResult<TopologySpec> => { /* pure */ }
```

Why this is better than a builder:

- pure values can be shared, versioned, diffed, and normalized
- composition order is explicit
- tests can assert exact topology content without mocking builders
- the same value can be sent to the control plane or stored in fixtures

### Open Locally, Closed Remotely

This is another deliberate design choice.

The API is:

- **open for local composition**
- **closed over remote executable kinds**

That means:

- anyone can compose topology values freely
- anyone can share reusable topology snippets as plain data
- but only component kinds the runtime knows how to interpret can appear in a
  `TopologySpec`

This is the right tradeoff for a substrate API.

If user code needs truly custom runtime behavior, there are only two honest
paths:

1. add a new named component kind plus runtime interpreter support
2. keep the custom logic local in ACP middleware or product-layer orchestration

What the substrate should **not** do is pretend an arbitrary TS closure can be
serialized and executed by the runtime.

### Runtime Spec

```ts
export type RuntimeSpec = {
  readonly provider: 'local' | 'docker' | string
  readonly host?: string
  readonly port?: number
  readonly name: string
  readonly agent_command: readonly string[]
  readonly topology?: TopologySpec
  readonly resources?: readonly ResourceRef[]
  readonly capabilities?: readonly CapabilityRef[]
}
```

The API stays substrate-level:

- no product workflow inputs
- no hidden defaults that invent a higher abstraction
- only portable launch-time concerns

## Module 2: `control` — Control-Plane Interpreter

The control module contains free functions that interpret core values against a
control-plane endpoint.

```ts
export type ControlPlane = {
  readonly base_url: string
  readonly headers?: Readonly<Record<string, string>>
}

export declare function provision(
  control: ControlPlane,
  spec: RuntimeSpec,
): Promise<RuntimeDescriptor>

export declare function getRuntime(
  control: ControlPlane,
  runtime_key: string,
): Promise<RuntimeDescriptor>

export declare function listRuntimes(
  control: ControlPlane,
  filter?: { readonly provider?: string; readonly status?: RuntimeDescriptor['status'] },
): Promise<readonly RuntimeDescriptor[]>

export declare function stopRuntime(
  control: ControlPlane,
  runtime_key: string,
): Promise<RuntimeDescriptor>

export declare function deleteRuntime(
  control: ControlPlane,
  runtime_key: string,
): Promise<void>
```

Design choice:

- use a plain `ControlPlane` value instead of a class
- keep lifecycle as free functions
- keep control plane explicit instead of hiding it inside a god-object

## Module 3: `acp` — Stateful ACP Interpreter

ACP is the one place where methods are appropriate because ACP is inherently
stateful.

```ts
export type AcpClient = {
  readonly endpoint: Endpoint
  newSession(input: { readonly cwd?: string }): Promise<{ readonly id: string }>
  loadSession(session_id: string, input: { readonly cwd?: string }): Promise<void>
  prompt(session_id: string, prompt: readonly AcpContent[]): Promise<void>
  subscribe(session_id: string): AsyncIterable<AcpEvent>
  close(): Promise<void>
}

export declare function connectAcp(endpoint: Endpoint): Promise<AcpClient>
```

This is a deliberate exception to the "values, not god-objects" rule:

- ACP is a live protocol
- connections have lifecycle
- sessions emit updates over time

Trying to force ACP into a purely methodless API would make it less honest.

### ACP Middleware Is Functional, Because It Stays Local

```ts
export type Layer<T> = (next: T) => T
export type AcpMiddleware = Layer<AcpClient>

export declare function chainAcp(
  ...middleware: readonly AcpMiddleware[]
): AcpMiddleware

export declare function withRetry(opts: {
  readonly attempts: number
}): AcpMiddleware

export declare function withTimeout(opts: {
  readonly ms: number
}): AcpMiddleware

export declare function withTracing(opts: {
  readonly sink: (event: AcpTraceEvent) => void
}): AcpMiddleware
```

This is where higher-order functions belong:

- they execute locally
- they are not serialized
- they are easy to verify in unit tests

This keeps the "same algebra" idea, but without pretending runtime topology and
client middleware are the same value kind.

## Module 4: `state` — Streams and Materializers

The state module owns the cold read side.

### Raw Stream Access

```ts
export type StreamCursor =
  | { readonly kind: 'beginning' }
  | { readonly kind: 'offset'; readonly offset: number | string }
  | { readonly kind: 'live' }

export declare function openStream(
  endpoint: Endpoint,
  opts?: { readonly from?: StreamCursor },
): AsyncIterable<StateEvent>
```

### Materializers

```ts
export type Materializer<S, E = StateEvent> = {
  readonly initial: S
  reduce(state: S, event: E): S
}

export type MaterializedHandle<S> = {
  snapshot(): S
  subscribe(fn: (state: S) => void): Unsubscribe
  close(): Promise<void>
}

export declare function materialize<S>(
  endpoint: Endpoint,
  materializer: Materializer<S>,
): Promise<MaterializedHandle<S>>

export declare function product<A, B>(
  a: Materializer<A>,
  b: Materializer<B>,
): Materializer<{ readonly a: A; readonly b: B }>
```

Built-in stores are just materializers:

```ts
export declare const sessionStore: Materializer<SessionIndex>
export declare const runtimeStore: Materializer<RuntimeIndex>
export declare const artifactStore: Materializer<ArtifactIndex>
```

Why this shape fits the mapping doc:

- materializers are folds over Session
- the hot/cold split stays explicit
- downstream product code can compose materializers without inventing a product
  server

## Module 5: `orchestration` — Staged Composition, Not Magic

This is where the earlier proposal needs the biggest correction.

The substrate does not yet justify a magical public `resume(sessionId)` that
implies "cold restart and continue semantic agent state" in all cases.

The better API is **staged** and explicit.

### Resume Context

```ts
export type ResumeContext = {
  readonly session_id: string
  readonly runtime_key: string
  readonly session: SessionRecord
  readonly runtime_spec?: PersistedRuntimeSpec
  readonly supports_load_session: boolean
}

export declare function readResumeContext(
  state: Endpoint,
  session_id: string,
): Promise<ResumeContext>
```

### Runtime Ensure

```ts
export type EnsureRuntimeResult = {
  readonly runtime: RuntimeDescriptor
  readonly created: boolean
}

export declare function ensureRuntimeForSession(
  control: ControlPlane,
  ctx: ResumeContext,
): Promise<EnsureRuntimeResult>
```

### Session Reattach

```ts
export type SessionLoadStatus =
  | { readonly kind: 'loaded' }
  | { readonly kind: 'not_supported' }
  | { readonly kind: 'not_found' }
  | { readonly kind: 'skipped' }

export declare function tryLoadSession(
  acp: AcpClient,
  ctx: ResumeContext,
): Promise<SessionLoadStatus>
```

### One-Shot Helper

```ts
export type ResumeOutcome = {
  readonly context: ResumeContext
  readonly runtime: RuntimeDescriptor
  readonly created: boolean
  readonly session_load: SessionLoadStatus
}

export declare function resumeSession(
  control: ControlPlane,
  shared_state: Endpoint,
  session_id: string,
): Promise<ResumeOutcome>
```

Why this is better than a single opaque `resume()`:

- it matches the actual composition described in
  [`managed-agents-mapping.md`](./managed-agents-mapping.md)
- it does not hide the distinction between "runtime is available" and
  "downstream agent successfully reattached semantic session state"
- each step can be tested independently
- higher-level products can wrap it with stronger guarantees if they own a more
  capable harness contract

## Supporting Types

Several domain types are referenced but not expanded in full here:

- `ContextSourceRef`
- `FileBackendRef`
- `StateEvent`
- `SessionRecord`
- `PersistedRuntimeSpec`
- `SessionIndex`
- `RuntimeIndex`
- `ArtifactIndex`
- `AcpContent`
- `AcpEvent`
- `AcpTraceEvent`
- `ValidationResult<T>`
- `Unsubscribe`

They are omitted only for brevity. The important design point is their role in
the algebra, not their exact field list in this proposal.

## End-to-End Example

```ts
import {
  audit,
  contextInjection,
  approvalGate,
  registerTool,
  composeTopology,
  provision,
  connectAcp,
  materialize,
  product,
  sessionStore,
  artifactStore,
  resumeSession,
  type ControlPlane,
} from '@fireline/client'

const control: ControlPlane = {
  base_url: 'http://localhost:3000',
}

const topology = composeTopology(
  audit({ env: 'dev' }),
  contextInjection([{ kind: 'workspace_files', path: '/work' }]),
  approvalGate({ scope: 'tool_calls', timeout_ms: 60_000 }),
  registerTool({
    name: 'review_pr',
    description: 'Review a pull request',
    input_schema: {
      type: 'object',
      properties: { url: { type: 'string' } },
      required: ['url'],
    },
  }),
)

const runtime = await provision(control, {
  provider: 'docker',
  name: 'reviewer',
  agent_command: ['codex'],
  topology,
  resources: [
    { kind: 'git_remote', repo_url: 'https://github.com/acme/repo', mount_path: '/work' },
  ],
})

const acp = await connectAcp(runtime.acp)
const session = await acp.newSession({ cwd: '/work' })
await acp.prompt(session.id, [{ type: 'text', text: 'Review the current PR' }])

const state = await materialize(
  runtime.state,
  product(sessionStore, artifactStore),
)

console.log(state.snapshot().a)

const resumed = await resumeSession(control, runtime.state, session.id)
console.log(resumed.runtime.runtime_key, resumed.session_load.kind)
```

This is still unmistakably functional:

- topology is pure data
- materializers are pure reducers
- ACP middleware is pure composition over a live client
- orchestration is explicit composition, not a product object

## Why This Design Is Better

### 1. It keeps the wire contract honest

The runtime cannot execute arbitrary TS closures. A data-first `TopologySpec`
keeps the public API aligned with what the runtime can actually consume.

### 2. It preserves composability

You still get:

- tiny constructor functions like `audit()` and `approvalGate()`
- pure composition via `composeTopology()`
- shareable topology snippets
- pure materializer composition via `product()`
- pure ACP middleware composition via `chainAcp()`

The API stays functional without pretending everything is one higher-order
function.

### 3. It separates primitive truth from portability/deployment concerns

`ToolDescriptor` is the Tools primitive.
`CapabilityRef` is a portable launch-time attachment.

That distinction matters both conceptually and operationally.

### 4. It makes verification easier

Pure values can be:

- snapshotted in fixtures
- diffed in golden tests
- validated before execution
- normalized deterministically

This is much harder with builder chains and impossible with arbitrary closures.

### 5. It keeps the hot/cold split visible

The mapping doc explicitly wants:

- hot ACP traffic
- cold replay/materialization surfaces

This design makes that split structural:

- `acp` is hot and stateful
- `state` is cold and replayable
- `control` is lifecycle/configuration

### 6. It gives higher-level products the right substrate

Products want to compose:

- domain-specific topology presets
- provisioning workflows
- resumer loops
- dashboards and materialized read models

They do not want to inherit substrate-level confusion about what is a
serializable spec versus what is a local interpreter function.

## Comparison to the Existing Two Proposals

| Question | Namespace proposal | Function-valued proposal | This proposal |
|---|---|---|---|
| Composition is first-class | Weak | Strong | Strong |
| Topology is serializable | Yes | No | Yes |
| Hot/cold split is visible | Strong | Weaker | Strong |
| Easy to diff/validate | Medium | Weak | Strong |
| User can write local pure logic | Medium | Strong | Strong |
| Runtime boundary stays honest | Medium | Weak | Strong |

## Recommendation

Adopt this as the new direction for the low-level TypeScript surface:

1. **Use data-first constructors for runtime-executed substrate values.**
2. **Use pure function composition for local interpreters only.**
3. **Keep orchestration staged and typed until stronger resume semantics exist.**
4. **Keep Tools primitive descriptors separate from capability portability refs.**
5. **Prefer logical modules over a single god-object, with optional umbrella re-exports.**

## Next Step

The smallest useful spike is:

1. implement `core` factories for `audit`, `contextInjection`, `approvalGate`,
   `registerTool`, and `composeTopology`
2. serialize a `TopologySpec` through the existing runtime launch path
3. implement `validateTopology()` as a pure function
4. implement `state.materialize()` on top of the existing `packages/state`
   substrate
5. implement staged `resumeSession()` returning `ResumeOutcome`, not a stronger
   implicit guarantee

That would prove the right abstraction level quickly without overcommitting to
an oversized TS SDK.
