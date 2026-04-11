# Fireline TypeScript API — Functional Proposal

> Status: proposal, not yet adopted
> Type: design doc
> Audience: maintainers deciding the public TypeScript surface
> Source: derived from [`./managed-agents-mapping.md`](./managed-agents-mapping.md) §"Fireline as combinators over the primitives"
> Related:
> - [`./managed-agents-mapping.md`](./managed-agents-mapping.md) — six primitives + seven-combinator algebra
> - [`../ts/low-level-api-surface.md`](../ts/low-level-api-surface.md) — current namespace-based proposal (this doc proposes an alternative shape)

## Why this doc exists

The current TypeScript surface proposal in `low-level-api-surface.md` is organized around **namespaces** (`client.host`, `client.acp`, `client.state`, `client.stream`, `client.topology`, `client.orchestration`). That's a familiar shape and matches how most TS SDKs are structured.

But the algebraic decomposition in `managed-agents-mapping.md` showed that everything Fireline does above the six primitives is **functional composition over seven combinators**. Topology is `compose(component, component, component, ...)`. Tools are init-time `mapEffect`. Resources are init-time Components with a single-fire constraint. Materializers are folds over the Session event log.

If the underlying model is composition, the API can be much smaller and more direct than a set of namespaces. This doc proposes that shape.

## Design principles

1. **Composition is the primary verb.** Every non-trivial operation builds something composable. Components compose into Topologies. Materializers compose into multi-projection stores. ACP middleware composes into a wrapped client.
2. **Values, not god-objects.** No `client.x.y.z()` chains. Free functions return immutable values that you pass around.
3. **The seven combinators are first-class.** Anyone can write a custom Component using `mapEffect`, `appendToSession`, `filter`, `substitute`, `suspend`, `fanout`, or `observe` directly. Built-in components are sugar; the combinators are the contract.
4. **Same algebra both sides of the wire.** Server-side proxy chains and client-side ACP middleware use the same `Component` type. Symmetric.
5. **No new abstractions beyond the six primitives.** Every type in this API maps to one or two primitives. If a proposed type doesn't, it doesn't belong in `@fireline/client`.

## Module layout

The proposal is one package — `@fireline/client` — with the following named exports. No god-object. No nested namespaces deeper than one level.

```typescript
// @fireline/client
export {
  // The seven base combinators (functional core)
  compose,
  observe,
  mapEffect,
  appendToSession,
  filter,
  substitute,
  suspend,
  fanout,

  // Built-in component presets (sugar over the combinators)
  audit,
  contextInjection,
  budget,
  approvalGate,
  peer,
  smithery,
  durableTrace,

  // Sandbox lifecycle (Sandbox primitive — provision side)
  provision,
  listRuntimes,
  getRuntime,
  stopRuntime,
  deleteRuntime,

  // ACP transport (Sandbox.execute + Harness I/O)
  connectAcp,
  // Client-side ACP combinators (mirror of the server-side seven)
  withRetry,
  withTimeout,
  withTracing,

  // Session reads (Session primitive — materializer folds)
  sessionStore,
  runtimeStore,
  materialize,
  openStream,

  // Orchestration (the wake primitive)
  wake,
  listWakes,

  // Types — values, not interfaces hidden behind methods
  type Component,
  type Topology,
  type RuntimeSpec,
  type RuntimeDescriptor,
  type ResourceRef,
  type CapabilityRef,
  type Endpoint,
  type WakeReason,
  type WakeReceipt,
  type Materializer,
} from '@fireline/client'
```

That's the entire public surface. ~30 exports total. Compare this to the current namespace-based proposal which has six namespaces with 25–40 methods between them.

## The seven combinators in TypeScript

Each combinator is a free function that returns a `Component`. A `Component` is a transformer over `Harness`. The base type:

```typescript
type Effect = AcpRequest | InitEffect      // server-side: anything the harness yields
type EffectResult = AcpResponse | InitOk   // server-side: anything that flows back
type Harness = (e: Effect) => Promise<EffectResult>
type Component = (next: Harness) => Harness
```

The seven combinators:

```typescript
// 1. observe — pass through, side-effect to an external sink
declare const observe: (sink: (e: Effect, r: EffectResult) => void) => Component

// 2. mapEffect — rewrite Effect before passing through
declare const mapEffect: (fn: (e: Effect) => Effect) => Component

// 3. appendToSession — pass through, write an event to the Session log
declare const appendToSession: (mk: (e: Effect, r: EffectResult) => SessionEvent) => Component

// 4. filter — reject Effects matching a predicate, return a substitute result
declare const filter: (
  pred: (e: Effect) => boolean,
  reject: (e: Effect) => EffectResult,
) => Component

// 5. substitute — rewrite one Effect into a different Effect
declare const substitute: (rewrite: (e: Effect) => Effect) => Component

// 6. suspend — pause Effect, write a wait record, resume on wake
declare const suspend: (reason: (e: Effect) => SuspendReason | null) => Component

// 7. fanout — turn one Effect into many, collect results
declare const fanout: (
  split: (e: Effect) => Effect[],
  merge: (rs: EffectResult[]) => EffectResult,
) => Component
```

Plus `compose`:

```typescript
// compose — fold a list of components into a single component (left-to-right)
declare const compose: (...components: Component[]) => Component

// And the identity component for starting compositions:
declare const identity: Component  // = (next) => next
```

## Topology is just `Component`

There is no separate `Topology` type. A `Topology` is a `Component`, and `compose` produces them. The naming is just a convenience:

```typescript
type Topology = Component  // a fully-composed harness transformer
```

This means a Topology can be passed anywhere a Component can, and vice versa. Composing topologies with more components is just `compose(topology, moreComponents...)`.

## Building a topology

The most functional shape:

```typescript
import {
  compose,
  audit,
  contextInjection,
  budget,
  approvalGate,
  peer,
  smithery,
} from '@fireline/client'

const myTopology = compose(
  audit(),
  contextInjection({ sources: [{ kind: 'workspace_files', path: '/work' }] }),
  budget({ tokens: 1_000_000 }),
  approvalGate({ scope: 'tool_calls', timeout_ms: 60_000 }),
  peer({ peers: ['runtime:reviewer', 'runtime:writer'] }),
  smithery({ catalog: 'https://smithery.ai/...', tools: ['review_pr'] }),
)
```

That's the entire topology builder. No `.builder()` method, no `.with()` chain, no `.attach()` calls. Just `compose` and the components you want.

The order is left-to-right: `audit` runs first on outgoing effects (sees them raw), `smithery` runs last (any tools it adds are visible to all earlier components).

## Built-in components are pure sugar over the seven combinators

Every built-in component is exported as a free function that returns a `Component` value. Each one decomposes to a known combinator pattern:

```typescript
// audit — sugar for appendToSession
export const audit = (opts?: AuditOpts): Component =>
  appendToSession((e, r) => ({
    kind: 'audit',
    effect: e,
    result: r,
    timestamp: now(),
    ...opts?.metadata,
  }))

// contextInjection — sugar for mapEffect
export const contextInjection = (opts: ContextInjectionOpts): Component =>
  mapEffect(e => isPrompt(e)
    ? { ...e, prompt: addContext(e.prompt, opts.sources) }
    : e)

// budget — sugar for filter
export const budget = (opts: BudgetOpts): Component =>
  filter(
    e => bucket(opts.tokens).tryConsume(estimateTokens(e)),
    e => ({ kind: 'error', error: 'budget exceeded', detail: e }),
  )

// approvalGate — sugar for suspend
export const approvalGate = (opts: ApprovalGateOpts): Component =>
  suspend(e => {
    if (opts.scope === 'tool_calls' && !isToolCall(e)) return null
    return {
      kind: 'approval',
      effect: e,
      timeout_ms: opts.timeout_ms ?? 60_000,
    }
  })

// peer — sugar for substitute + tool registration via mapEffect
export const peer = (opts: PeerOpts): Component =>
  compose(
    // Register the peer tools at init time
    mapEffect(e => e.kind === 'init'
      ? { ...e, tools: [...e.tools, ...peerTools(opts.peers)] }
      : e),
    // Rewrite peer tool calls to ACP requests against the peer runtimes
    substitute(e => isPeerToolCall(e)
      ? toPeerAcpCall(e, opts.peers)
      : e),
  )

// smithery — sugar for mapEffect over init
export const smithery = (opts: SmitheryOpts): Component =>
  mapEffect(e => e.kind === 'init'
    ? { ...e, tools: [...e.tools, ...resolveSmithery(opts.catalog, opts.tools)] }
    : e)

// durableTrace — sugar for appendToSession on both directions
export const durableTrace = (): Component =>
  appendToSession((e, r) => ({ kind: 'trace', effect: e, result: r }))
```

Reading the source of any built-in tells you exactly what it does in terms of the seven combinators. There's no hidden behavior. If you want to write your own, copy the pattern.

## Custom components

Writing a custom component is composition over the seven combinators — no special framework, no extension API:

```typescript
import { mapEffect, filter, appendToSession, compose, type Component } from '@fireline/client'

// Tag every effect with a request ID
const requestIdTagger: Component = mapEffect(e => ({
  ...e,
  meta: { ...e.meta, requestId: generateId() },
}))

// Rate-limit by token bucket
const rateLimit = (rps: number): Component => {
  const bucket = makeTokenBucket(rps)
  return filter(
    () => bucket.tryAcquire(),
    () => ({ kind: 'error', error: 'rate limited' }),
  )
}

// Slack notify on every tool call
const slackNotify = (webhook: string): Component => appendToSession(
  (e, r) => {
    if (isToolCall(e)) {
      void fetch(webhook, { method: 'POST', body: JSON.stringify({ tool: e.tool }) })
    }
    return { kind: 'slack_notified', tool: isToolCall(e) ? e.tool : null }
  },
)

// Compose them with built-ins
const myTopology = compose(
  audit(),
  requestIdTagger,
  rateLimit(10),
  slackNotify('https://hooks.slack.com/...'),
  approvalGate({ scope: 'tool_calls' }),
)
```

The user-written components are first-class. They compose with built-ins, with each other, with anything else of type `Component`. There is no "extension API" because there's nothing to extend — everything is already the same kind of thing.

## Sandbox lifecycle

Provision is a free function that takes a `RuntimeSpec` and returns a `RuntimeDescriptor`. The topology is just a field on the spec:

```typescript
import { provision, type RuntimeSpec } from '@fireline/client'

const runtime = await provision({
  agent: { command: 'codex' },
  placement: { provider: 'docker', image: 'fireline/runtime:latest' },
  topology: myTopology,
  resources: [
    { kind: 'git', repo_url: 'https://github.com/...', mount_path: '/work' },
  ],
  capabilities: [
    {
      kind: 'tool_ref',
      tool: {
        name: 'review_pr',
        description: 'Review a GitHub PR',
        input_schema: { /* ... */ },
        transport_ref: { kind: 'mcp_url', url: 'https://...' },
        credential_ref: { kind: 'env', var: 'GITHUB_TOKEN' },
      },
    },
  ],
})

// runtime: RuntimeDescriptor — a plain data value, not a class
console.log(runtime.runtimeKey)
console.log(runtime.acp.url)    // Endpoint
console.log(runtime.state.url)  // Endpoint
```

Other lifecycle operations are also free functions:

```typescript
import { listRuntimes, getRuntime, stopRuntime, deleteRuntime } from '@fireline/client'

const all = await listRuntimes({ provider: 'docker' })
const r = await getRuntime('runtime:abc')
await stopRuntime('runtime:abc')
await deleteRuntime('runtime:abc')
```

`RuntimeDescriptor` is a plain data type. No methods. No hidden state. You can serialize it to JSON and pass it around freely.

## ACP transport

`connectAcp` takes an `Endpoint` (the descriptor's `acp` field) and returns an `AcpClient`. The client is the only place in this API that has methods, because ACP itself is a stateful protocol:

```typescript
import { connectAcp } from '@fireline/client'

const acp = await connectAcp(runtime.acp)
const session = await acp.newSession({ /* ... */ })
await session.prompt('Review the PR at https://...')

session.events.subscribe(event => {
  if (event.kind === 'agent_message_chunk') {
    process.stdout.write(event.text)
  }
})

await session.respondToPermission(requestId, { optionId: 'allow' })
```

But the same combinator algebra works on the **client side** of ACP too. The seven combinators have a dual that operates on `AcpClient → AcpClient`:

```typescript
import { connectAcp, withRetry, withTimeout, withTracing, observeAcp } from '@fireline/client'

const acp = withRetry({ attempts: 3 })(
  withTimeout({ ms: 30_000 })(
    withTracing({ sink: console.log })(
      await connectAcp(runtime.acp),
    ),
  ),
)
```

Or via `compose` — the same `compose` works because the algebra is the same shape:

```typescript
import { compose, withRetry, withTimeout, withTracing } from '@fireline/client'

const acpMiddleware = compose(
  withRetry({ attempts: 3 }),
  withTimeout({ ms: 30_000 }),
  withTracing({ sink: console.log }),
)

const acp = acpMiddleware(await connectAcp(runtime.acp))
```

This gives Fireline a **symmetric story**: the SAME algebra runs on both sides of the wire. Server-side combinators run inside the runtime via the conductor proxy chain. Client-side combinators run in the TS process via the AcpClient wrapper. Same patterns, two locations.

## Session reads — materializers

Materializers are folds over the Session event log. The base type:

```typescript
type Materializer<S> = {
  initial: S
  reduce: (state: S, event: SessionEvent) => S
}
```

A materializer is a value. To run it against a runtime's stream:

```typescript
import { materialize, type Materializer } from '@fireline/client'

// Define a materializer
const sessionCount: Materializer<number> = {
  initial: 0,
  reduce: (n, e) => e.kind === 'session_started' ? n + 1 : n,
}

// Run it against a runtime stream
const handle = materialize(runtime.state, sessionCount)
const currentCount = handle.snapshot()  // current value
handle.subscribe(n => console.log('count:', n))
await handle.close()
```

Built-in materializers are presets — values, not classes:

```typescript
import { sessionStore, runtimeStore, materialize } from '@fireline/client'

const sessions = materialize(runtime.state, sessionStore)
const session = sessions.get('session:abc')
const all = sessions.list()
sessions.subscribe(updates => { /* reactive */ })
```

Composing two materializers into a product is a free function:

```typescript
import { product, materialize } from '@fireline/client'

const combined = product(sessionStore, runtimeStore)
const handle = materialize(runtime.state, combined)
const { sessions, runtimes } = handle.snapshot()
```

Same algebra: composition over a small base. No god-object. No "store" classes with hidden state. Materializers are values that get folded over an event stream.

## Raw stream access (Session primitive, low level)

For consumers that want to subscribe to the durable stream directly without materializing:

```typescript
import { openStream } from '@fireline/client'

const stream = openStream(runtime.state, { from: 'live' })
for await (const event of stream) {
  console.log(event.offset, event.kind)
}

// Or replay from a cursor
const replay = openStream(runtime.state, { from: { offset: 0 } })
for await (const event of replay) {
  process(event)
}
```

`openStream` returns an async iterator. No methods, no classes. The stream is a value that yields events.

## Orchestration — wake

`wake` is a single free function:

```typescript
import { wake, type WakeReason } from '@fireline/client'

const reason: WakeReason = {
  kind: 'webhook',
  webhookId: req.body.eventId,
  payload: req.body,
}

const receipt = await wake('runtime:slack-bot', reason)
console.log(receipt.wakeId, receipt.willInstantiate)
```

That's the entire Orchestration namespace. One function. The receipt is a plain data value.

For listing past wakes (for debugging or operator visibility):

```typescript
import { listWakes } from '@fireline/client'

const recent = await listWakes({ runtimeKey: 'runtime:slack-bot', since: Date.now() - 60_000 })
```

## End-to-end happy path

The whole API in one example:

```typescript
import {
  // Combinators + presets
  compose, audit, contextInjection, budget, approvalGate, peer, durableTrace,
  // Sandbox
  provision,
  // ACP
  connectAcp,
  // Session reads
  sessionStore, materialize,
  // Orchestration
  wake,
} from '@fireline/client'

// 1. Build the topology — pure value, no side effects yet
const topology = compose(
  audit(),
  contextInjection({ sources: [{ kind: 'workspace_files', path: '/work' }] }),
  budget({ tokens: 1_000_000 }),
  approvalGate({ scope: 'tool_calls' }),
  peer({ peers: ['runtime:reviewer'] }),
  durableTrace(),
)

// 2. Provision a sandbox
const runtime = await provision({
  agent: { command: 'codex' },
  placement: { provider: 'docker' },
  topology,
  resources: [{ kind: 'git', repo_url: 'https://github.com/...', mount_path: '/work' }],
})

// 3. Connect ACP and run a session
const acp = await connectAcp(runtime.acp)
const session = await acp.newSession({})
await session.prompt('Review the PR at https://...')

// 4. Read materialized session state
const sessions = materialize(runtime.state, sessionStore)
console.log(sessions.list())

// 5. Later, from a webhook handler — wake the runtime
await wake(runtime.runtimeKey, {
  kind: 'webhook',
  webhookId: 'evt-123',
  payload: { /* ... */ },
})
```

Every value in this example is plain data or a composable function. There is no `client` god-object. There are no classes with hidden state. Each step is a free function call returning a value you can pass around.

## What this collapses

Compared to the current `low-level-api-surface.md` namespace-based proposal:

| Current (namespace-based) | Functional proposal |
|---|---|
| `client.host.create(spec)` | `provision(spec)` |
| `client.host.list()` | `listRuntimes()` |
| `client.host.get(key)` | `getRuntime(key)` |
| `client.host.stop(key)` | `stopRuntime(key)` |
| `client.acp.connect(endpoint)` | `connectAcp(endpoint)` |
| `client.state.open({endpoint})` | `materialize(endpoint, sessionStore)` |
| `client.stream.open(endpoint)` | `openStream(endpoint)` |
| `client.topology.builder().attach('audit')...` | `compose(audit(), ...)` |
| `client.topology.attachTool({...})` | `mapEffect(e => addTool(e, tool))` |
| `client.orchestration.wake(key, reason)` | `wake(key, reason)` |

The functional shape removes the `client.` prefix, removes the namespace nesting, and replaces builder chains with `compose`. Same operations, fewer concepts.

## What this preserves

- **Six primitives + seven combinators** as the underlying contract
- **No `client.runs` / `client.workspaces` / `client.profiles`** — those are product objects, not substrate
- **Endpoint, RuntimeDescriptor, SessionDescriptor** as the core data types
- **The control plane / data plane split** — `provision` and `wake` go to the control plane; `connectAcp`, `openStream`, `materialize` go to the data plane
- **Symmetry between server-side proxy chains and client-side ACP middleware**
- **Tools and Resources as Components**, not separate concepts

## What this makes possible

Two things become trivial under this shape that are awkward under the namespace-based shape:

### 1. Topology snippets are shareable values

Because a Topology is just a Component (which is just a function), you can share, version, and import topologies the same way you share any TypeScript value:

```typescript
// my-org/agent-topologies/coding.ts
import { compose, audit, contextInjection, approvalGate, peer } from '@fireline/client'

export const codingTopology = compose(
  audit(),
  contextInjection({ sources: [{ kind: 'workspace_files', path: '/work' }] }),
  approvalGate({ scope: 'tool_calls' }),
  peer({ peers: ['runtime:reviewer'] }),
)

// In another package
import { compose, budget } from '@fireline/client'
import { codingTopology } from '@my-org/agent-topologies'

const myTopology = compose(
  codingTopology,           // import a base
  budget({ tokens: 500_000 }),  // add my own constraints
)
```

This is impossible under a builder-based API because builders carry state. Pure functional values compose freely.

### 2. Same code on both sides of the wire

The same `compose` and the same algebra works for server-side proxy chains AND client-side ACP middleware. A library author can ship a single component that works in both locations:

```typescript
// my-org/fireline-tracing/index.ts
import { observe, type Component } from '@fireline/client'

export const tracing = (sink: TracingSink): Component => observe((e, r) => {
  sink.record({ effect: e, result: r, timestamp: now() })
})

// Use server-side
const topology = compose(audit(), tracing(consoleSink), approvalGate({...}))

// Use client-side (same component, different wrapper)
const acp = compose(withRetry({...}), tracing(consoleSink))(await connectAcp(runtime.acp))
```

## What's harder under this shape

There are real tradeoffs. I'm flagging them so the reaction can be informed.

1. **Discoverability via autocomplete is worse.** `client.<TAB>` shows you six namespaces. With named exports, you have to know what to import. Mitigation: clear documentation, IDE auto-imports, a small enough surface that it fits in one mental model.

2. **The seven combinators have generic types that might confuse beginners.** `Component = (next: Harness) => Harness` is a higher-order function over a higher-order function. Some users will find this harder than `client.topology.attach('audit')`. Mitigation: built-in components hide the combinator types entirely. Beginners use `audit()`, `contextInjection()`, etc. — the seven combinators are for advanced users writing custom components.

3. **The `connectAcp` return value is the one place with methods.** ACP is a stateful protocol — `session.prompt()` mutates the session, `session.events.subscribe()` returns a subscription. This is unavoidable and right; it's an island of statefulness inside an otherwise functional API. Document it as such.

4. **No central `client` object means no central configuration.** If a downstream user wants to set a default base URL, default auth, etc., there's no one place to do that. Mitigation: add a `configure()` free function that sets module-level defaults, or accept config as the first arg of each function. The namespace-based proposal has the same problem; this isn't worse.

5. **Migration cost from any existing code that uses `client.host.*` etc.** This is a real cost. Can be mitigated by shipping the functional API alongside the namespace API for a transition period, then deprecating the namespace API.

## Open questions

1. **Does the runtime side need to mirror this exactly?** The Rust conductor today builds the proxy chain via topology spec. If the TS surface is `compose(...)`, the spec that travels over the wire could be a serialized component graph rather than a flat list of named components. That has implications for how user-written components get deployed (they don't, unless they're known by name). Defer until first cross-runtime-component is requested.

2. **How do parameters travel?** A `Component` is a closed-over function. The serialization that goes to the runtime needs to identify which built-in component plus its config, not the function itself. Means the public API has named built-ins (`audit()` returns a tagged value, not just a function). The seven base combinators need a way to identify their config too — probably each returns a `Component & { kind: string; config: unknown }` value so it can be serialized.

3. **Where does the seven-combinator implementation live?** Inside `@fireline/client` (TS), which then serializes to a topology spec consumed by the Rust conductor? Or does the Rust conductor have its own implementation and the TS side just emits config? Probably both — TS is for client-side composition (where the runtime is the executor), Rust is for server-side execution (where the runtime is also the loop runner). The wire format is the contract.

4. **Materializers — TS-only or also Rust?** Today TS `StreamDB` and Rust `RuntimeMaterializer` both exist. Under this proposal, they're the same shape (`Materializer<S>`). Worth aligning their APIs explicitly so users can write a materializer once and it runs in either location.

5. **Should `compose` be variadic, or take an array?** Variadic is more ergonomic for hand-written code; an array is easier when you're dynamically building topologies. Maybe both: `compose(...components)` and `compose.from(components)`.

## Recommendation

I think this is the right shape for Fireline's TS API once the substrate-first reframe is the source of truth. It is materially smaller than the namespace-based proposal (~30 exports vs. 6 namespaces with 25–40 methods), it makes the combinator algebra discoverable and usable, and it gives users a path to write their own components without an extension API.

But it is a meaningful change from the current `low-level-api-surface.md` direction, which is itself only days old. Before adopting:

1. **React to the shape.** Does this feel right? Anywhere it feels wrong?
2. **Decide whether to replace `low-level-api-surface.md` or run them as alternatives.** Replacing is cleaner; running both creates ambiguity.
3. **Pin the migration path.** If we adopt this, what happens to existing code that imports from `client.host.*`? Aliases for one release, hard break in the next, or full parallel APIs?
4. **Pin the wire format question** (open question 3 above) before any code lands.

If the shape is right, the next step is a small spike: pick one built-in component (probably `audit` since it's the simplest), implement it as a `Component` value backed by `appendToSession`, plumb it through the existing runtime topology spec to confirm the wire format works, and then incrementally rebuild the rest from there.
