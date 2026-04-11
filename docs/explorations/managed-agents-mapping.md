# Fireline against Anthropic's Managed-Agent Primitives

> Status: **operational source of truth** for Fireline's substrate roadmap
> Type: reference + decision + execution-driving doc
> Audience: maintainers deciding what to build, in what order, against what acceptance bars
> Source: Anthropic engineering blog, *"Managed agents: a small set of primitives for any agent harness"* (https://www.anthropic.com/engineering/managed-agents)
> Related:
> - [`../proposals/client-primitives.md`](../proposals/client-primitives.md) — the authoritative TypeScript substrate surface, built on the six primitives this doc names
> - [`../proposals/runtime-host-split.md`](../proposals/runtime-host-split.md) §7 — Host / Sandbox / Orchestrator reframe grounding the Rust-side trait layout against the same primitives
> - [`../proposals/crate-restructure-manifest.md`](../proposals/crate-restructure-manifest.md) — target Rust crate layout aligned 1:1 with the primitive taxonomy
> - [`./managed-agents-citations.md`](./managed-agents-citations.md) — file:line inventory of where each primitive is implemented today

## How to read this doc

This is the **source of truth** for what Fireline should build, in what order, and against what acceptance bars. The substrate-shape proposals in [`../proposals/`](../proposals/) and the Rust-side trait layout all derive from the primitive framing here.

If you're picking up new work, start here. If you're writing a new proposal or slice, cite this doc by section heading and pick a primitive to anchor against. If a feature doesn't fit any primitive, that's a signal the shape is wrong — it may belong in a downstream product, not in Fireline.

## Purpose

Anthropic's managed-agents post defines a minimal abstraction layer for "what makes an agent harness managed": six interfaces that any implementation must satisfy, each with trivial example implementations (Postgres, cron job, while-loop, local process, S3, MCP server).

This doc maps Fireline onto those six interfaces. It answers four questions:

1. **Which primitives does Fireline already implement well?**
2. **Which primitives is Fireline missing, and what would it cost to add them?**
3. **What is the build order and acceptance bar for each primitive?**
4. **How does the existing slice plan line up against this framework?**

The framework is valuable because it gives us shared vocabulary across three different conceptual stacks — Anthropic's managed-agents post, the Flamecast RFCs, and Rivet AgentOS — that all converge on the same underlying shape. If Fireline names its surfaces in this vocabulary, downstream consumers (Flamecast, future products) can plug into them without translation.

This doc does **not** propose new primitives that aren't in the Anthropic framework. The whole point is to constrain Fireline's surface to a small canonical set instead of inventing new abstractions for every product capability.

## The six primitives at a glance

| # | Primitive | Interface (pseudocode) | Satisfied by | Fireline status |
|---|---|---|---|---|
| 1 | **Session** | `getSession(id) → (Session, Event[])`; `getEvents(id) → PendingEvent[]`; `emitEvent(id, event)` | Any append-only log consumed in order from any event point with idempotent appends | **Strong on live Rust-side invariants; slice 14 read-surface work remains** (all five managed-agent Session clauses are live in `tests/managed_agent_session.rs:52`, `:126`, `:213`, `:415`, and `:523`; the remaining work is canonical row schema + TS read-surface hardening, not Session semantics — see §1 below) |
| 2 | **Orchestration** | `wake(session_id) → void` | Any scheduler that can call a function with an ID and retry on failure | **Compositionally correct on the Rust side; TS/client surface still pending** (the cold-start acceptance contract is live at `tests/managed_agent_primitives_suite.rs:132`, and the live-runtime no-op / concurrent-wake / subscriber-loop proofs are live at `tests/managed_agent_orchestration.rs:84`, `:161`, and `:249`; `@fireline/client` still does not ship `wake(sessionId)` — see §2 below) |
| 3 | **Harness** | `yield Effect<T> → EffectResult<T>` | Any loop that yields effects and appends progress to the Session | **Partial** (by design; durable suspend/resume also satisfied via composition once Orchestration is wired up) |
| 4 | **Sandbox** | `provision({resources}) → execute(name, input) → String` | Any executor configured once and called many times as a tool | **Strong** |
| 5 | **Resources** | `[{source_ref, mount_path}]` | Any object store the container can fetch from by reference | **Strong on ACP-fs and component-layer mounts; Docker-scoped shell-mount proof stays external** (`managed_agent_resources_physical_mount_acceptance_contract`, `managed_agent_resources_fs_backend_acceptance_contract`, and `managed_agent_resources_fs_backend_component_test` are live at `tests/managed_agent_primitives_suite.rs:249`, `:308`, and `:369`; runtime-level fs-backend and cross-runtime stream-backed file tests are live at `tests/managed_agent_resources.rs:195` and `:287`; the only ignored row is the intentional Docker-scoped cross-reference marker at `tests/managed_agent_resources.rs:154` / `tests/managed_agent_primitives_suite.rs:290` — see §5) |
| 6 | **Tools** | `{name, description, input_schema}` | Any capability describable as a name and input shape | **Strong on live Rust-side descriptor invariants; portable refs still pending** (schema-only, transport-agnostic, and deterministic first-attach-wins coverage is live at `tests/managed_agent_tools.rs:63`, `:230`, and `:389`; the remaining work is launch-spec portability and call-time credential resolution — see §6 below) |

**One-line summary:** Sandbox remains the strongest operational primitive (`tests/managed_agent_sandbox.rs:58`, `:109`, `:185`). Session is now fully green on the Rust-side managed-agent invariants (`tests/managed_agent_session.rs:52`, `:126`, `:213`, `:415`, `:523`), with slice 14 still owning canonical row schema + TS read-surface hardening. Harness is still honestly partial: the missing seam is the durable suspend/resume round trip at `tests/managed_agent_harness.rs:326`. Tools now have live schema-only / transport-agnostic / collision invariants (`tests/managed_agent_tools.rs:63`, `:230`, `:389`). Orchestration has live cold-start / no-op / race / subscriber-loop coverage on the Rust side (`tests/managed_agent_primitives_suite.rs:132`; `tests/managed_agent_orchestration.rs:84`, `:161`, `:249`), but `@fireline/client` still does not ship `wake(sessionId)`. Resources are live at the component layer and for ACP-fs / cross-runtime stream-backed file behavior (`tests/managed_agent_primitives_suite.rs:249`, `:308`, `:369`; `tests/managed_agent_resources.rs:195`, `:287`); the remaining ignored mount row is an intentional Docker-scoped cross-reference marker, not pending local-runtime work (`tests/managed_agent_resources.rs:154`; `tests/managed_agent_primitives_suite.rs:290`). **Net result: the remaining primitive-level gap is the Harness durable suspend/resume seam plus downstream TS/product-facing surfaces, not missing substrate basics.**

## Fireline as combinators over the primitives

Fireline introduces concepts above Anthropic's minimal six — conductor components, proxy chains, materializers, the topology spec — but **none of these are new primitives**. They are all functional compositions over the existing six. This section gives the algebraic decomposition.

Anyone proposing a new conductor component should be able to express it as one of the combinators below or a small composition of them. If they cannot, that's a signal: either Fireline needs a new primitive (so far only Orchestration and Resources have qualified) or the feature belongs in a downstream product layer, not in the substrate.

### A conductor component is `Harness → Harness`

The Harness primitive is `Effect → EffectResult`. A conductor component is a higher-order function that takes a harness and returns a wrapped harness:

```typescript
type Harness   = (e: Effect) => Promise<EffectResult>
type Component = (next: Harness) => Harness
```

This is the standard middleware shape — Tower in Rust, Express middleware in Node, Connect handlers, Polka, Hapi extensions, Tower's `Layer`, the `decorator` pattern. Components compose via standard function composition:

```typescript
const compose = (...components: Component[]): Component =>
  (next) => components.reduceRight((acc, c) => c(acc), next)
```

`client.topology` is `[Component]`. Building the topology is `compose`. The runtime takes the composed function and uses it as its proxy chain. Your example — `context_injection(peer(audit(Effect)))` — is exactly this composition: each component wraps the next, the result is a single transformed `Harness`.

### Seven combinators cover every Fireline component today

There are exactly seven base combinators that all current Fireline conductor components decompose into. Each is parameterized by which primitive(s) it touches.

| # | Combinator | Type signature | Touches primitive | Example use |
|---|---|---|---|---|
| 1 | `observe(sink)` | `(e: Effect) => void` → `Component` | external sink | logging, metrics export |
| 2 | `mapEffect(fn)` | `(e: Effect) => Effect` → `Component` | Harness only | context injection, prompt template rewriting |
| 3 | `appendToSession(mk)` | `(e: Effect) => Event` → `Component` | **Session** | audit, durable trace |
| 4 | `filter(pred, reject)` | `(e: Effect) => bool` × `() => EffectResult` → `Component` | Harness only | budget gate, policy block |
| 5 | `substitute(rewrite)` | `(e: Effect) => Effect` → `Component` | Harness only | peer call routing, tool dispatch |
| 6 | `suspend(reason)` | `(e: Effect) => SuspendReason` → `Component` | **Session** + **Orchestration** | approval gate, durable wait |
| 7 | `fanout(split, merge)` | `(e: Effect) => Effect[]` × `(rs: EffectResult[]) => EffectResult` → `Component` | Harness (asymmetric n→1) | parallel tool calls, multi-peer dispatch |

Six base + fanout for parallelism. Combinators 1, 2, 4, 5, 7 touch only the Harness primitive (they're pure transformers). Combinator 3 writes to the Session primitive. Combinator 6 writes to the Session primitive AND registers a wake handler with the Orchestration primitive — that's why Orchestration is the load-bearing missing piece for suspend/resume to be meaningful.

### How current Fireline components decompose

| Component | Combinator decomposition |
|---|---|
| `AuditTracer` | `appendToSession(e => ({kind: 'audit', effect: e}))` |
| `DurableStreamTracer` | `appendToSession(e => ({kind: 'trace', effect: e, result: r}))` — bidirectional, also captures result on the way back |
| `ContextInjectionComponent` | `mapEffect(e => addContext(e, sources))` |
| `BudgetComponent` | `filter(e => budget.check(e), () => ({error: 'budget exceeded'}))` |
| `ApprovalGateComponent` | `suspend(e => isToolCall(e) ? {kind: 'approval', tool: e.tool} : null)` — gates only on tool calls, passes prompts through |
| `PeerComponent` | `substitute(e => isPeerCall(e) ? toPeerEffect(e) : e)` plus tool registration via `mapEffect` |
| `SmitheryComponent` | tool registration via `mapEffect(e => e.kind === 'init' ? {...e, tools: [...e.tools, ...smitheryTools]} : e)` |

Every existing Fireline component is one combinator or a small composition. None of them require a new primitive. If a future component cannot be written as a combinator, that's a signal worth investigating — it usually means either a missing primitive (rare) or that the feature is composing too many concerns and should be split into smaller components (common).

### Tools are also Components

A Tools registration is just a `mapEffect` that adds an item to the Effect's `available_tools` set on the init Effect:

```typescript
const registerTool = (tool: ToolSpec): Component =>
  mapEffect(e => e.kind === 'init'
    ? { ...e, tools: [...e.tools, tool] }
    : e)
```

This means **Tools and Components are the same kind of thing**. The Tools primitive is a special case of `mapEffect` over the init Effect. `client.topology.attachTool({...})` is sugar for `compose(currentTopology, registerTool({...}))`. A list of tools is a list of init-time Components.

### Resources are also Components

Resources are nominally a launch-spec field, but algebraically they're a Component that fires once on the init Effect:

```typescript
const provision = (resources: ResourceRef[]): Component =>
  (next) => async (e) => {
    if (e.kind === 'init') {
      for (const r of resources) {
        await mount(r.source_ref, r.mount_path)
      }
    }
    return next(e)
  }
```

Resources fire once per Sandbox lifetime, on the init Effect. They're an init-time Component with a single-fire constraint. The TS-side surface exposes them as a launch-spec field for ergonomics, but the underlying shape is still a Component composed into the proxy chain at provision time.

### Materializers are folds over the Session event log

Outside the proxy chain, **materializers** are a different combinator family that operates on the Session event log directly rather than on the live effect path. They are pure folds:

```typescript
type Materializer<S> = (event: Event, state: S) => S
```

Each materializer is a fold step: given an event and the current state, return the new state. The Session is the source list, the materializer is the fold function, the result is derived state. `SessionIndex`, `RuntimeMaterializer`, and TS `StreamDB` all fit this exact shape.

Materializers compose via product:

```typescript
const productMat = <A, B>(ma: Materializer<A>, mb: Materializer<B>): Materializer<{a: A, b: B}> =>
  (e, {a, b}) => ({ a: ma(e, a), b: mb(e, b) })
```

So Fireline has **two layers of pure functional folds** sitting on top of the six Anthropic primitives:

```text
┌────────────────────────────────────────────────────────────┐
│  Materializer pipeline                                     │
│  fold of (Event, S) → S over the Session event log         │
│  produces: derived state for queries (SessionIndex, etc.)  │
└────────────────────────────────────────────────────────────┘
                            ▲
                            │ reads
                            │
┌────────────────────────────────────────────────────────────┐
│  Session primitive                                         │
│  append-only log + idempotent appends                      │
└────────────────────────────────────────────────────────────┘
                            ▲
                            │ writes (via appendToSession)
                            │
┌────────────────────────────────────────────────────────────┐
│  Conductor proxy chain                                     │
│  fold of Component transformers over the base Harness      │
│  produces: a wrapped Harness with the topology behaviors   │
└────────────────────────────────────────────────────────────┘
                            ▲
                            │ wraps
                            │
┌────────────────────────────────────────────────────────────┐
│  Harness primitive                                         │
│  Effect → EffectResult                                     │
└────────────────────────────────────────────────────────────┘
```

Both layers are pure functional folds. Both compose. Both decompose into operations on the existing primitives. Neither requires Fireline to introduce a new abstraction beyond the seven combinators above plus the materializer fold.

### The Anthropic round-trip

This decomposition lets us round-trip cleanly between Fireline's complex shape and Anthropic's minimal shape. Anyone reading the Anthropic post should be able to point at any Fireline feature and ask "which primitive plus which combinator?" and get a single-sentence answer:

| Fireline feature | Anthropic primitive | Combinator / composition |
|---|---|---|
| `compose(audit(), ...)` | Session + Harness | `appendToSession . mapEffect` |
| `compose(approvalGate(), ...)` | Session + Harness | `suspend` (writes a pending event, rebuilds via `session/load` on resume) |
| `compose(smithery(), peer(), ...)` (tool registration) | Tools + Harness | `mapEffect` over init |
| `provision({ resources: [...] })` | Resources + Harness | `provision` (init-time Component) |
| `provision({ topology })` | Sandbox + Harness | `compose` of all topology components |
| `sessionStore.get(id)` | Session | materializer fold |
| `openStream(endpoint, cursor)` | Session | identity fold (raw passthrough) |
| `wake(sessionId)` | Session + Sandbox + Harness | composition: `sessionStore.get` + `provision` + `connectAcp` + `loadSession` — **no standalone wake primitive** |
| subscriber loop watching a runtime stream and calling `wake` | Orchestration | *"any loop that appends to a log and calls a function with retry"* — satisfied by `for await (event of openStream(...))` + `wake` |
| `acp.session.prompt(...)` | Sandbox.execute + Harness yield | direct passthrough |

Everything Fireline does is reducible to "primitive(s) + combinator". This is the operational answer to "what belongs in Fireline vs. what belongs in Flamecast." Fireline is the substrate that provides the six primitives plus the seven-combinator algebra. Flamecast composes those into product objects (runs, workspaces, profiles, approval queues) that don't fit the combinator algebra and shouldn't.

### Why this matters operationally

When proposing a new Fireline feature, run it through this test:

1. **Express it as a combinator first.** If it's a new conductor component, decompose it into the seven combinators. If it's a materializer, write it as a `(Event, S) → S` fold step.
2. **If you cannot decompose it, be suspicious.** A feature that doesn't fit the combinator algebra is reaching for something the substrate doesn't have. Either Fireline needs a new primitive (unusual — only Orchestration and Resources have qualified so far) or the feature is a product object that belongs in Flamecast.
3. **If it composes cleanly, you are probably right.** The combinator algebra is the test for "does this fit Fireline's shape" before any code is written.

This is also the test for whether a feature is *too small* to belong in Fireline. If a proposed component is "just `mapEffect(addHeader)`", that's a one-liner; it's a configuration, not a component. Fireline ships components that have meaningful internal state or non-trivial composition; one-liners belong in user code.

## 1. Session — Strong on live invariants; slice 14 read surface remains

### Anthropic interface

```text
getSession(session_id) → (Session, Event[])
getEvents(session_id) → PendingEvent[]   // not yet processed
emitEvent(id, event)
```

Satisfied by *"any append-only log that can be consumed in order from any event point and accepts idempotent appends — Postgres, SQLite, in-memory array, etc."*

### What Fireline exposes

Fireline's Session implementation is the **durable-streams server + per-runtime trace stream**. The `durable-streams` upstream runs unchanged, one deployment per environment, accepting one stream per runtime keyed by `runtime_key`. Inside that stream, every `TraceEvent` is an append-only entry with a stable offset, broadcast over SSE to subscribers.

The pieces:

- **Producer side (`emitEvent`)** — `fireline_conductor::trace::DurableStreamTracer` (`crates/fireline-conductor/src/trace.rs`) wraps a `durable_streams::Producer` and writes one event per ACP and conductor activity. Idempotent appends are guaranteed by the producer wire protocol.
- **Consumer side (`getEvents` / `getSession`)** — two consumers exist today:
  - **Runtime-local materializers** (`fireline_conductor::state_projector`, `crates/fireline-conductor/src/runtime/...`) that subscribe to the runtime's own stream and project rows for `SessionIndex`, `ActiveTurnIndex`, etc.
  - **TypeScript `StreamDB`** in `packages/state/` that the browser uses for reactive queries against the same stream.
- **Replay-from-any-point** — `durable-streams` SSE subscriptions accept `offset` and `live` parameters, so any consumer can start at any point in history. The runtime restart story works because per-runtime streams persist independently of compute.
- **Architectural commitment** — the [control-and-data-plane doc §3b](../runtime/control-and-data-plane.md) explicitly names durable-streams as the *persistence tier of the data plane*, with materialization happening only in consumers, never in the persistence tier.

### Gap

No substrate-shape gap. The persistence tier exists, the replay protocol exists, the producer and at least two consumer kinds exist, the architectural role is committed.

The Rust managed-agent suite now covers the five concrete Session clauses Anthropic cares about:

1. **Append-only replay from offset 0** — `tests/managed_agent_session.rs:52`
2. **Durability across runtime death** — `tests/managed_agent_session.rs:126`
3. **Replay from an arbitrary captured offset** — `tests/managed_agent_session.rs:213`
4. **Idempotent append under retry** — `tests/managed_agent_session.rs:415`
5. **Materialized-vs-raw agreement** — `tests/managed_agent_session.rs:523`

The remaining work is now about the **read surface**, not the Session primitive itself:

1. **Stabilizing the canonical row schema** so downstream products can read the same Session events without forking — slice 14's main job.
2. **Surfacing replay/catch-up semantics ergonomically in TypeScript** so downstream consumers do not have to reverse-engineer the persistence protocol from Rust internals.

### How existing slices contribute

- **Already shipped:** durable-streams integration, `DurableStreamTracer`, runtime-local materializers, TS `StreamDB`
- **Slice 14 (in plan)** — canonical row schema, replay/catch-up TypeScript surface, distinction between hot ACP traffic and cold read-oriented state
- **Slice 13b (just shipped)** — added the runtime descriptor and registration shape that Session events reference

## 2. Orchestration — Composable (no new primitive needed)

### Anthropic interface

```text
wake(session_id) → void
```

Satisfied by *"any scheduler that can call a function with an ID and retry on failure — a cron job, a queue consumer, a while-loop, etc."*

### The reduction

An earlier version of this doc treated Orchestration as Fireline's biggest gap, recommending a new slice 18 to introduce a `wake(runtime_key)` HTTP endpoint and an in-process scheduler. On closer inspection **the primitive is already satisfied by composition of existing surfaces**:

- **`durable-streams` accepts writes from any authenticated client**, not just from the runtime. Per [`control-and-data-plane.md`](../runtime/control-and-data-plane.md) §3b, the write surface is the standard durable-streams HTTP POST; nothing is gating external appends beyond the bearer token the control plane mints. An external process with a stream-write token can `emitEvent` to a runtime's Session log as freely as the runtime can.
- **Any process that can `openStream` becomes a scheduler.** A subscriber is exactly *"any loop that can call a function with an ID and retry on failure"* — the "function" is `emitEvent` or `wake(sessionId)`, the "ID" is the session_id from the event, and retry semantics fall out of the fact that the subscriber can re-consume the stream from its last processed offset on restart.
- **`session/load` already rebuilds session state from durable evidence.** `src/load_coordinator.rs` exposes `LoadCoordinatorComponent` taking a `SessionIndex` — a materialized view over the Session log — and reconstructing ACP session state when a client reconnects. It is event-sourcing the session.
- **`RuntimeHost::create` can cold-start a runtime for a stored spec.** Provided the spec is durably persisted (write it into the Session log at provision time, read it back at wake time), any process with control-plane credentials can instantiate a fresh runtime against the same `runtime_key`.

Composing these four things gives the canonical `wake` pattern:

```typescript
// The ENTIRE Orchestration primitive, expressed as composition
async function wake(sessionId: string) {
  // 1. Look up session → runtime mapping from the Session read surface (slice 14)
  const session = await sessionStore.get(sessionId)

  // 2. If the runtime is dormant or killed, cold-start from the stored spec
  let runtime = await getRuntime(session.runtimeKey)
  if (!runtime || runtime.status !== 'ready') {
    runtime = await provision(session.runtimeSpec)   // RuntimeHost::create
  }

  // 3. Rebuild the ACP session state from the durable log
  const acp = await connectAcp(runtime.acp)
  await acp.loadSession(sessionId)                    // existing session/load
}
```

Ten lines. No scheduler service. No new HTTP endpoint. No new Rust primitive. The "scheduler" is a subscriber loop:

```typescript
// Any process can run one of these. Flamecast runs one. A webhook receiver runs one.
// An approval service runs one. A cron-triggered batch runner runs one.
const stream = openStream(runtime.state, { from: 'live' })
for await (const event of stream) {
  if (event.kind === 'approval_resolved' && event.allow) {
    await wake(event.sessionId)
  }
}
```

### The flow, end-to-end

Walk through an out-of-band approval, which is the hardest case (session may be dormant, runtime may be killed, wake trigger is external):

1. Agent yields a `tools/call` effect that requires approval
2. `ApprovalGateComponent` intercepts the effect and writes a `PermissionRequest` event to the Session log, returning "pending" to the agent
3. Runtime's job is done for now; it may be torn down to save cost (or killed by the operator, or crashed)
4. External approval service subscribes to the relevant Session streams via `openStream`
5. Service sees the `PermissionRequest` event, pings the human (Slack, email, whatever)
6. Human approves
7. Service appends an `ApprovalResolved { allow: true }` event to the Session log via direct durable-streams POST
8. A "waker" subscriber (same service or a separate process) sees the `ApprovalResolved` event and calls `wake(sessionId)`
9. `wake` checks if the runtime is live: it's not (torn down in step 3)
10. `wake` calls `provision(session.runtimeSpec)` to cold-start the runtime
11. Runtime comes up, `session/load` rebuilds the ACP session state from the Session log
12. On rebuild, `ApprovalGateComponent` sees the recent `ApprovalResolved` event matching its pending `PermissionRequest` and releases the pause
13. Agent's effect resumes and advances

Every step uses an existing Fireline primitive. The only composition glue that doesn't exist yet is:

- The `wake` helper itself (a ~10-line TS function)
- The `runtimeSpec` being durably persisted alongside session metadata so `wake` can retrieve it (part of slice 14)
- The `ApprovalGateComponent`'s "on rebuild, scan the log for pending resolutions" behavior (small addition to an existing component)

### Why this reduction works

The thing that tripped up the earlier framing is that Anthropic's `wake(session_id) → void` sounded like it wanted a **single entry point** — a function you call to advance a specific session. The reduction is realizing that the entry point already exists: it's `emitEvent` to the Session log. Any event that a runtime-local component treats as "time to advance" becomes a wake trigger, and the runtime comes back to life via `wake` in response.

This matches Anthropic's stated framing — *"any scheduler that can call a function with an ID and retry on failure — a cron job, a queue consumer, a while-loop, etc."* The scheduler isn't a new service; it's anything that can subscribe to a durable log and call a function. Fireline's subscribers (materializers, external services, operator tools, other runtimes' components) are all satisfying the primitive already.

### What's still needed (and what isn't)

**Still needed, small:**

- `wake(sessionId)` helper in `@fireline/client` — TS-side composition of `sessionStore.get` + `getRuntime` + `provision` + `connectAcp` + `loadSession`. Ships as part of the TS API surface work, not a Rust slice.
- `runtimeSpec` durable persistence — add it to the Session log as an event at provision time (or to a small control-plane catalog). Part of slice 14's canonical read schema work.
- `ApprovalGateComponent` rebuild behavior — on `session/load`, scan recent events for pending resolutions. Small addition to the existing component, not a new component.
- A worked example of the subscriber pattern in docs — how to run a "waker" loop, how to handle coordination between multiple subscribers, how to claim work via a stream event to avoid duplicate resumes.

**Not needed anymore:**

- Slice 18 as originally scoped (new `/v1/runtimes/{key}/wake` HTTP endpoint, in-process scheduler service, new Rust primitive). The scheduler is anything that subscribes; the entry point is `emitEvent` + `wake`; the retry semantics fall out of subscription offset tracking.
- `client.orchestration.wake` as its own TS namespace. The wake operation is `wake(sessionId)` — a composition helper, not a primitive.
- A new `WakeReason` type with variants for webhook, timer, approval, peer. Each of those triggers is just an event on the Session stream with its own `kind`, no special primitive.

### How existing slices contribute

- **Already shipped:** `durable-streams` writes from any authenticated producer; runtime registration + heartbeat; `DurableStreamTracer` producing events; `LoadCoordinatorComponent` and `session/load` rebuilding session state; `RuntimeHost::create` cold-starting runtimes.
- **Slice 13c (in flight)** — proves cold-start works for non-local providers, which is what `wake` exercises.
- **Slice 14 (planned)** — canonical session read schema that `wake` relies on (session → runtime_key → runtimeSpec mapping).
- **Slice 16 (reframed)** — out-of-band approvals become the FIRST CONSUMER of the `wake` pattern, not a new primitive. The work is: upgrade `ApprovalGateComponent` to rebuild from the log on `session/load`, ship a worked example of the waker subscriber loop, document the coordination patterns.
- **Slice 18 (deleted)** — Orchestration doesn't need its own slice. The work folds into slice 14 (durable spec persistence), slice 16 (approval component upgrade), and the TS API surface (`wake` helper).

## 3. Harness — Partial (by design)

### Anthropic interface

```text
yield Effect<T> → EffectResult<T>
```

Satisfied by *"any loop that yields effects and appends progress to the Session."*

### What Fireline exposes

Fireline doesn't own the loop. The harness is the agent process — Claude Code, Codex, fireline-testy, or any ACP-speaking subprocess — and its loop is what yields effects (tool calls, MCP requests, model completions). Fireline sits *between* the harness and its effects via the **conductor proxy chain**.

The pieces:

- **`PromptResponderProxy`** intercepts `session/prompt` requests; **`PromptObserverProxy`** observes them; **`MessageObserver`** components see message chunks
- **Topology composition** lets users register components that wrap, transform, observe, or substitute effects — `AuditTracer`, `ContextInjectionComponent`, `ApprovalGateComponent`, `BudgetComponent`, `SmitheryComponent`, `PeerComponent`
- **`DurableStreamTracer`** persists every effect into the Session log, satisfying the "appends progress to the Session" half of the contract

So Fireline serves the Harness primitive at a different layer than Anthropic's framing assumes — Anthropic models the harness as an opaque loop that the substrate calls; Fireline treats the harness's I/O as a programmable proxy chain. **Both are valid.** Fireline's choice is more flexible because it lets components compose around the loop without owning the loop.

The proxy chain is not a new abstraction. Algebraically, it's a `compose` over a small set of `Harness → Harness` transformers that decompose into seven combinators (`observe`, `mapEffect`, `appendToSession`, `filter`, `substitute`, `suspend`, `fanout`). Every Fireline component today is one combinator or a small composition. See [§Fireline as combinators over the primitives](#fireline-as-combinators-over-the-primitives) above for the full algebraic decomposition.

### Gap

The gap is **suspend/resume**. Fireline can intercept an effect mid-flight (`ApprovalGateComponent` does exactly this — it pauses the prompt response until approval lands), but it can't currently *persist the harness's continuation across runtime death and resume it from a new process*. The current `ApprovalGateComponent` works because the runtime stays alive while waiting; if the runtime dies, the pause is lost.

This is the same gap as Orchestration. Without `wake`, there's nowhere for a resumed harness to land. Once `wake` exists, the suspend/resume seam in the conductor becomes load-bearing rather than convenient.

### How existing slices contribute

- **Already shipped:** topology component registry, all five tier-1 components, `DurableStreamTracer`
- **Slice 18 (proposed)** — the wake primitive that makes durable suspend/resume meaningful
- **Component depth (later slices)** — richer approval, budget, routing, delegation components on the same proxy chain seam

## 4. Sandbox — Strong

### Anthropic interface

```text
provision({resources}) → execute(name, input) → String
```

Satisfied by *"any executor that can be configured once and called many times as a tool — a local process, a remote container, etc."*

### What Fireline exposes

`RuntimeProvider::start(spec) → RuntimeLaunch` is `provision`. The implementations:

- **`LocalProvider`** (`crates/fireline-conductor/src/runtime/local.rs`) — local subprocess, ships today
- **`ChildProcessRuntimeLauncher`** (`crates/fireline-control-plane/src/local_provider.rs`) — the control-plane-backed version with `prefer_push: bool` from 13b
- **`DockerProvider`** (slice 13c, in flight) — bollard-backed Docker container
- **Future:** E2B, Daytona, Cloudflare, Kubernetes — same trait

### Where Fireline diverges from Anthropic's shape

The Anthropic interface is `execute(name, input) → String`: a synchronous tool invocation. Fireline's runtime is a long-lived ACP server, not a single `execute()` call. The runtime *contains* the execution loop rather than being called per-input.

This is a deliberate choice and the right one for stateful agents. ACP sessions accumulate context, the harness has its own loop, MCP tool calls happen inside that loop. Reducing this to `execute(name, input) → String` would require turning every prompt into a separate sandbox invocation, losing the in-process context entirely.

The reconciliation: **Fireline's Sandbox is `provision()` plus a long-lived ACP server, and the ACP session is the per-input call channel.** Anthropic's `execute()` corresponds to one ACP `session/prompt` request. The sandbox is configured once (via `provision`), called many times (via `session/prompt`), and torn down on completion.

### Gap

None at the substrate level. The trait exists, two implementations work, a third is in flight. The remaining work is provider depth — more `RuntimeProvider` impls — which doesn't change the primitive shape.

### How existing slices contribute

- **Already shipped:** `RuntimeProvider`, `LocalProvider`, `ChildProcessRuntimeLauncher`, push lifecycle from 13b
- **Slice 13c (in flight)** — `DockerProvider` via bollard
- **Future slices:** E2B, Daytona, Cloudflare, Kubernetes providers — additive, no contract change

## 5. Resources — Partially composable (ACP fs interception) + small physical-mount gap

> **Related eval:** [`stream-fs-resources-evaluation.md`](./stream-fs-resources-evaluation.md) — a parallel evaluation of Durable Streams' experimental `stream-fs` package as a potential Resources backend. The eval concludes that `stream-fs` is NOT the right first answer for Resources v1 and should be deferred in favor of the generic `ResourceRef + ResourceMounter + LocalPathMounter + GitRemoteMounter` path described below. `stream-fs` may become a later narrow spike as a read-only pinned snapshot mount via FUSE once the generic primitive ships.
>
> **Disambiguation warning:** `SessionLogFileBackend` (Fireline's single-writer artifact-log-as-filesystem described below) is a materially different design from upstream `stream-fs` (a general collaborative filesystem with many writers, metadata streams, rename semantics, and watch/SSE coordination). The names are dangerously close. The Fireline backend is scoped to a single runtime's own Session stream, supports flat path→latest content only, and uses the stream offset as its revision identity — none of the concerns the eval raises about `stream-fs` apply to it in this constrained form.

### Anthropic interface

```text
[{source_ref, mount_path}]
```

Satisfied by *"any object store the container can fetch from by reference — Filestore, GCS, a git remote, S3."*

### The reduction

An earlier version of this doc treated Resources as fully missing. Closer inspection shows it splits cleanly into two halves — one of which is composable over the existing combinator algebra, and the other of which needs a small targeted addition for one real-world constraint (shell-based agents).

**The composable half: ACP file system interception.**

The ACP protocol defines [`fs/read_text_file` and `fs/write_text_file`](https://agentclientprotocol.com/protocol/file-system) as client-hosted methods. The runtime serves these. Because they flow through the conductor proxy chain as ACP requests, the seven-combinator algebra applies directly:

```typescript
// An FsBackendComponent is compose(substitute, appendToSession)
const fsBackend = (backend: FileBackend): Component => compose(
  substitute(e =>
    isFsRead(e)  ? { ...e, resolve: () => backend.read(e.path) } :
    isFsWrite(e) ? { ...e, resolve: () => backend.write(e.path, e.content) } :
    e
  ),
  appendToSession((e, r) =>
    isFsOp(e)
      ? { kind: 'fs_op', op: opKind(e), path: e.path, result: r }
      : null
  ),
)
```

Where `FileBackend` is a small trait with pluggable implementations: `LocalFileBackend`, `S3FileBackend`, `GcsFileBackend`, `GitFileBackend`, `SessionLogFileBackend`.

Three things follow for free:

1. **Backend is a configuration choice, not a new primitive.** Pointing the runtime at S3 instead of local disk is one component attach.
2. **Artifact persistence is automatic.** Every `fs/write_text_file` is both routed to the backend AND appended to the Session log via `appendToSession`. A materializer over `fs_op` events becomes "what files did this run produce, and where did they land."
3. **Session log can BE the backend.** `SessionLogFileBackend` stores file content as events and reads via projection. The Session log IS the filesystem — durable by construction, replayable, queryable, cross-runtime-observable. Elegant for small workflows; impractical for large binary-heavy ones.

**The non-composable half: shell-based agents bypass ACP fs.**

Claude Code, Codex, and most real agents use bash/python/their own internal tools to read and write files. A bash `cat /work/src/main.rs` is an opaque ACP `tools/call` that returns a string — we see the result, but the actual read happened inside the container's filesystem without passing through `fs/read_text_file`. Shell is Turing-complete; we can't reliably intercept every file operation.

So for shell-based agents the files must physically exist on the container's filesystem before the agent starts. This means: **inbound `source_ref → mount_path` still needs a physical mount at provision time.** That's what slice 15's `ResourceMounter` trait is for, and we can't compose our way out of it.

### What's actually needed

| Piece | Status |
|---|---|
| `resources: Vec<ResourceRef>` field on `CreateRuntimeSpec` | **Missing** — slice 15 |
| `ResourceMounter` trait on runtime provider side | **Missing** — slice 15 |
| `LocalPathMounter` (bind mount) | **Missing** — slice 15, likely ships as 13c side effect |
| `GitRemoteMounter` (clone + checkout) | **Missing** — slice 15 |
| `S3Mounter` / `GcsMounter` | **Missing** — slice 15 follow-ups |
| `FsBackendComponent` with `FileBackend` trait | **Composable** — one conductor component, no new primitive |
| `LocalFileBackend`, `S3FileBackend`, `GcsFileBackend`, `GitFileBackend`, `SessionLogFileBackend` | **Composable** — backend implementations layered under the component |
| Session log as artifact record | **Already works** — falls out of `appendToSession` on writes |

### Why the two halves complement each other

A single runtime can run both layers simultaneously:

- **Physical mount at `/work`** — git repo cloned in for shell-based file access via bash/python/etc.
- **`FsBackendComponent` for ACP fs ops** — any agent that uses `fs/read_text_file` or `fs/write_text_file` (or any MCP tool backed by ACP fs) gets routed through the component
- **Artifact capture via `appendToSession`** — every ACP-native write is logged, regardless of backend

Shell-based reads of the physical mount and ACP-native reads of the virtual backend coexist. Artifacts that the agent produces via `fs/write_text_file` land in the chosen backend AND the Session log. Artifacts produced via shell (e.g., `echo 'x' > /tmp/out.txt`) are invisible to the component — that's a known limitation of shell-based agents, and the mitigation is to configure the agent to use ACP fs or MCP file tools for anything that needs to be persisted.

### What slice 15 actually ships

Slice 15 shrinks from "full Resources product with workspace object" to **two focused deliverables**:

1. **Physical mounts (Rust side, ~1 week):** `ResourceRef` type, `ResourceMounter` trait, `LocalPathMounter` and `GitRemoteMounter` implementations, `CreateRuntimeSpec.resources` field, provider wiring to invoke mounters at start time.
2. **`FsBackendComponent` (Rust + TS, ~1–2 days):** the conductor component, the `FileBackend` trait, `LocalFileBackend` and `SessionLogFileBackend` as the first two implementations, TS-side `resources` helpers. S3/GCS/git backends land later.

Total slice 15 scope: ~1.5 weeks of work spanning Rust conductor, Rust provider, and TS helpers. Not a full execution slice in the old sense, but meaningful and self-contained.

### How existing slices contribute

- **Slice 13c (in flight)** — Docker provider needs to mount *something* into the container; a minimal `LocalPathMounter` will likely land here as a side effect. The component work waits for slice 15.
- **Slice 15 (reduced scope)** — physical `ResourceMounter` + `FsBackendComponent` + first two backends. Rewrites the existing slice 15 doc.
- **Future slices:** S3/GCS/git backends added incrementally as real consumers need them.

### One important unlock

Once `FsBackendComponent` ships, the `SessionLogFileBackend` special case becomes a really interesting primitive for small, durable, distributed workflows. Imagine:

- Two runtimes on different hosts, both pointed at the same Session stream
- Runtime A writes `/scratch/report.md` via `fs/write_text_file`
- The write is captured as an event on the shared Session stream
- Runtime B reads `/scratch/report.md` via `fs/read_text_file`
- The component queries the projection of the Session stream and returns runtime A's content

**A cross-runtime virtual filesystem, for free, built on the existing durable-streams infrastructure.** No new primitive, no shared storage other than the stream that's already persistent. This is the kind of composition win that makes the primitive algebra worth using — features fall out that weren't designed in.

## 6. Tools — Strong on live descriptor invariants; portable refs still pending

### Anthropic interface

```text
{name, description, input_schema}
```

Satisfied by *"any capability describable as a name and an input shape — MCP server, custom tool, etc."*

### What Fireline exposes

Tools are schema-only in Fireline's existing model:

- **`PeerComponent`** (`crates/fireline-components/src/peer/`) — injects MCP-server-shaped tools that proxy calls to peer runtimes
- **`SmitheryComponent`** (`crates/fireline-components/src/smithery.rs`) — injects tools from any Smithery MCP catalog entry by name
- **MCP injection via topology** — the conductor topology can register arbitrary MCP servers per session
- **Host-tool bridges via conductor proxies** — the proxy chain lets host code intercept or wrap tool calls

The transport is open: any MCP server, any custom Rust tool, any host-side bridge satisfies the contract.

### Gap

No substrate-level gap. The descriptor-level invariants are now live against a running runtime:

- **Schema-only descriptor surface** — `tests/managed_agent_tools.rs:63`
- **Transport-agnostic registration without wire leakage** — `tests/managed_agent_tools.rs:230`
- **Deterministic first-attach-wins collision rule** — `tests/managed_agent_tools.rs:389`
- **Acceptance-level schema-only sibling in the primitives suite** — `tests/managed_agent_primitives_suite.rs:430`

That means the conductor/tooling layer is already proving the Anthropic wire contract on the Rust side. The remaining work is portability and credential indirection, not descriptor correctness.

The remaining work is **portable references** — letting a run carry "I want these tools mounted, fetched from these credential sources" rather than baking the tool list into the spawn arguments. That's slice 17 (capability profiles), reframed as:

- A capability profile is a list of `{name, description, input_schema, transport_ref, credential_ref}` entries
- `transport_ref` points to "where to fetch this tool" (Smithery URL, peer runtime key, MCP server endpoint)
- `credential_ref` points to "where to resolve auth at call time" (secret store path, environment binding, per-session OAuth token)

This keeps credentials out of the runtime and out of the spawn spec.

### How existing slices contribute

- **Already shipped:** `PeerComponent`, `SmitheryComponent`, topology MCP injection, conductor proxy chains
- **Slice 17 (in plan, reframed)** — capability profiles as portable Tools references with credential_ref indirection
- **External auth seam** (in priorities #5) — the credential_ref resolution layer

## No remaining primitive-sized gaps

An earlier version of this doc said Orchestration and Resources were the two real gaps. Both have since been reduced:

- **Orchestration** (§2) collapses into composition of Session subscribe + `session/load` + `RuntimeHost::create`, exposed as a ten-line `wake(sessionId)` helper. No slice 18, no new primitive, no scheduler service.
- **Resources** (§5) splits into two halves: ACP fs interception is pure composition via an `FsBackendComponent` (just `compose(substitute, appendToSession)`), and physical mounts for shell-based agents need a small focused addition via `ResourceMounter`.

### What's actually missing, sorted by size

**Small Rust additions / hardening:**

- `GitRemoteMounter` and any other non-local `ResourceMounter`s beyond the already-live `LocalPathMounter`
- `ApprovalGateComponent` rebuild-from-log behavior to close `tests/managed_agent_harness.rs:326`

**Small additions that fold into other slices:**

- Canonical row schema + TS replay/catch-up surface (slice 14, Session read surface)
- `@fireline/client` `wake(sessionId)` export / TS ownership of the orchestration helper
- Portable `CapabilityRef` / `credential_ref` launch inputs (slice 17)

**TS API surface work:**

- `wake(sessionId)` helper as a named export (tracked in `typescript-functional-api-proposal.md`)
- `fsBackend` component factory and `FileBackend` types

**Zero new primitives. Zero new slices. Zero new control-plane endpoints.**

That's the whole remaining *shape* gap list — nothing the substrate is missing at the primitive level. The acceptance bars below show where the *live coverage* is still genuinely thin: the approval-gate durable suspend/resume round trip for Harness, the TypeScript-owned `wake(sessionId)` helper for Orchestration, and non-local mounters beyond the already-live local path story. The Docker-scoped shell-visible mount check remains intentionally external to the local-runtime managed-agent suite. Once those targeted additions land, the substrate is complete; everything else is composition over the seven combinators plus product-layer work that belongs in Flamecast, not in the substrate.

## Build order and slice index

This is the operational plan: which slices ship in what order to close the gaps and harden the strong primitives. Each slice is tagged by which primitive it extends.

### Slice index, organized by primitive

| Primitive | Status | Slices that contribute | Status of those slices |
|---|---|---|---|
| **Session** | Strong on live invariants; slice 14 read surface remains | `14` Session as canonical read surface | The five Rust-side Session clauses are live at `tests/managed_agent_session.rs:52`, `:126`, `:213`, `:415`, and `:523`; slice 14 still owns canonical row schema + TS replay/catch-up surface |
| **Sandbox** | **Strong** (today's strongest — live `provision` + multi-execute tests) | `13a` control-plane runtime API; `13b` push lifecycle and auth; `13c` first remote provider (Docker via bollard) | 13a + 13b shipped on `main`; 13c in flight in workspace 7 |
| **Tools** | Strong on live descriptor invariants; portable refs still pending | `17` capability profiles as portable tool references | Schema-only / transport-agnostic / deterministic-collision invariants are live at `tests/managed_agent_tools.rs:63`, `:230`, and `:389`; slice 17 remains for launch-spec portability and call-time credential resolution |
| **Harness** | Honestly partial | `16` approval component rebuild behavior | The approval-gate-based durable suspend/resume round trip is the real missing piece — `tests/managed_agent_harness.rs:326` is `#[ignore]` |
| **Orchestration** | Compositionally correct on the Rust side; TS helper still pending | `16` approval component rebuild; `14` canonical read surface; `@fireline/client` ships the `wake(sessionId)` helper | The cold-start acceptance contract is live at `tests/managed_agent_primitives_suite.rs:132`; the live-runtime no-op / concurrent-wake / subscriber-loop proofs are live at `tests/managed_agent_orchestration.rs:84`, `:161`, and `:249`; TS `wake(sessionId)` is still not shipped |
| **Resources** | Strong on ACP-fs and component-layer mounts; Docker shell-mount proof intentionally external | `15` depth work for additional mounters and documentation | Component-layer mount / fs-backend proofs are live at `tests/managed_agent_primitives_suite.rs:249`, `:308`, and `:369`; launched-runtime fs-backend + cross-runtime stream-backed file proofs are live at `tests/managed_agent_resources.rs:195` and `:287`; only the Docker-scoped shell-visible mount marker stays ignored at `tests/managed_agent_resources.rs:154` / `tests/managed_agent_primitives_suite.rs:290` |

### Build order, with rationales

The order is chosen to maximize unblocking — every slice enables at least one downstream slice or primitive completion.

**1. `13c` Docker provider (Sandbox depth)** — *in flight, workspace 7*

First non-local provider. Forces the push lifecycle from 13b to be exercised end-to-end against a real container. Establishes the `RuntimeProvider` trait as the universal Sandbox boundary. It also owns the real container-filesystem proof that the shell-visible mount invariant is cross-referenced against from the local-runtime managed-agent suite.

**2. `14` Session as canonical read surface** — *next, can start in parallel with 13c*

Stabilizes the row schema downstream products read from the durable state stream: `runtime`, `session`, `prompt_turn`, `permission`, `terminal`, `chunks`, child-session edges. The `runtimeSpec` persistence needed for `wake` is already live; slice 14 now hardens the read contract and ships the TypeScript materialization layer that downstream consumers embed. Does not depend on 13c — they're orthogonal lanes.

**3. `15` Resources depth** — *small, can run in parallel with 13c and 14*

Continue the Resources rewrite from "Workspace object" to "Resources primitive." `resources: Vec<ResourceRef>`, `ResourceMounter`, `LocalPathMounter`, and the fs-backend path are already live; the remaining work is `GitRemoteMounter`, additional mounters, and clearer provider/mounter contract documentation. S3 and GCS mounters land later.

**4. `16` Out-of-band approvals + `wake` helper** — *first real Orchestration consumer*

Reframed from "approval product object" to the first worked example of the Orchestration composition pattern from §2 above. Two pieces:

- **`ApprovalGateComponent` rebuild behavior.** Upgrade the component so that on `session/load` it scans recent Session events for `ApprovalResolved` entries matching its pending `PermissionRequest`s and releases the pause accordingly. Small addition to an existing component.
- **`wake(sessionId)` helper in `@fireline/client`.** Ten-line TS composition: `sessionStore.get` → `getRuntime` → `provision` if dormant → `connectAcp` → `loadSession`. Ships as a named export alongside the rest of the TS API surface.

Plus a documented example of a "waker" subscriber loop and guidance on multi-subscriber coordination (how to claim a wake via a durable claim event so two subscribers don't duplicate work). **No new Rust primitive, no new control-plane endpoint, no slice 18.**

**5. `17` Capability profiles as portable Tools references** — *Tools depth*

Reframed from heavy "CapabilityProfile product object" to "portable launch input that bundles tool refs + credential refs + topology defaults." Shipped as a launch-spec field, similar to `15`'s collapse. Adds `credential_ref` indirection so credentials resolve at call time rather than spawn time.

**6. Component depth (ongoing)** — *Tools and Harness composition*

After the substrate primitives are in place, deepen the conductor components: stronger `BudgetComponent`, richer `ApprovalGateComponent`, new `RoutingComponent` for service delegation, new `DelegationComponent` for cross-runtime peer dispatch with retries. These are additive on top of the existing topology and don't require new primitives.

### What's NOT in the build order

- A "session product object" with REST CRUD endpoints — Session is a read surface, not a product object. Downstream products build that on top.
- A "workspace database" — Resources is a launch-spec field, not a managed database.
- A "capability profile catalog UI" — capability profiles are a portable launch input, not a registry product.
- A federated control plane / multi-region scheduler — out of scope until single-region works at scale.
- A peer-to-peer ACP proxy that the control plane sits in front of — peer ACP traffic is direct compute-to-compute by design.

If a proposed slice doesn't fit a primitive, that's a signal it belongs in a downstream product (Flamecast, the eventual `@fireline/*` consumer SDK), not in Fireline's substrate.

## Acceptance bars per primitive

These define what it means for a primitive to be "complete" enough to call itself stable. They are not gates on shipping individual slices — slices ship incrementally — but they're the bar a primitive must meet before downstream products can rely on it without escape hatches.

### Session — acceptance bar

- [x] Append-only durable log per runtime, replayable from any offset (durable-streams + `DurableStreamTracer`)
- [x] Idempotent append under retry is pinned to the durable-streams producer protocol and proven live at `tests/managed_agent_session.rs:415` (`session_idempotent_append_under_retry`)
- [x] At least one runtime-local consumer (`SessionIndex`, `RuntimeMaterializer`)
- [x] At least one external consumer (`packages/state` `StreamDB`)
- [ ] Canonical row schema documented and stable (slice 14)
- [ ] TypeScript materialization layer with replay/catch-up semantics that downstream products can embed (slice 14)
- [ ] Distinction between hot ACP traffic and cold read-oriented state called out in TS surface (slice 14)

**Status:** Strong on live invariants. Slice 14 still matters, but for canonical row schema and TS replay/catch-up ergonomics, not for the Session primitive's core semantics.

### Sandbox — acceptance bar

- [x] `RuntimeProvider` trait with `start()` returning a `RuntimeLaunch`
- [x] `LocalProvider` ships and works in dev mode
- [x] `ChildProcessRuntimeLauncher` ships as the control-plane-backed local provider
- [x] Push lifecycle (`/register`, `/heartbeat`) so providers don't need shared filesystem (slice 13b)
- [x] Bearer auth on push surface, scoped per `runtime_key` (slice 13b)
- [x] Live `provision` + multi-execute contract — `tests/managed_agent_sandbox.rs:54` (`sandbox_provision_returns_reachable_runtime`) and `tests/managed_agent_sandbox.rs:105` (`sandbox_provisioned_runtime_serves_multiple_execute_calls`) are Fireline's strongest managed-agent invariants today
- [ ] At least one non-local provider (slice 13c — Docker via bollard)
- [ ] Mixed local + non-local topology proof (slice 13c)
- [ ] Cross-runtime peer call traverses mixed topology with reconstructible lineage (slice 13c)

**Status:** ~80% complete and the strongest primitive today at the live-invariant level. Slice 13c closes the remaining items. Additional providers (E2B, Daytona, Cloudflare) are additive depth and don't gate completion.

### Tools — acceptance bar

- [x] Tools are described by `{name, description, input_schema}` (MCP-shape) at the conductor level
- [x] Topology component model lets tools be injected per session (`PeerComponent`, `SmitheryComponent`)
- [x] Conductor proxy chain lets host code intercept and wrap tool calls
- [x] Live invariant that the init effect surfaces tool descriptors as `{name, description, input_schema}` only, with no transport or credential leakage — `tests/managed_agent_tools.rs:63` (`tools_schema_only_contract`) and `tests/managed_agent_primitives_suite.rs:430` (`managed_agent_tools_schema_only_acceptance_contract`)
- [x] Live invariant that differently transported capabilities project the same schema-only wire shape with no transport or credential leakage — `tests/managed_agent_tools.rs:230` (`tools_transport_agnostic_registration`)
- [x] Live invariant that same-name collisions resolve deterministically via first-attach-wins — `tests/managed_agent_tools.rs:389` (`tools_first_attach_wins_on_name_collision`)
- [ ] Portable tool references in launch spec, with `credential_ref` indirection (slice 17)
- [ ] Credential resolution at call time, not spawn time (slice 17)

**Status:** Strong on Rust-side descriptor invariants. Slice 17 still matters for portable launch inputs and call-time credential resolution, but the Anthropic wire contract is already live.

### Harness — acceptance bar

- [x] Conductor proxy chain intercepts the harness's I/O (`PromptResponderProxy`, `PromptObserverProxy`, message observers)
- [x] Topology composition lets components wrap, transform, observe, or substitute effects
- [x] All harness progress is appended to the durable session log via `DurableStreamTracer` (`tests/managed_agent_harness.rs:61` + line 133 cover append and stable ordering)
- [x] `LoadCoordinatorComponent` rebuilds ACP session state from the durable log on `session/load`
- [ ] Conductor components can pause mid-effect and resume across runtime death by writing the pause as an event and rebuilding via `session/load` — pending, `tests/managed_agent_harness.rs:326` (`harness_durable_suspend_resume_round_trip`) is `#[ignore]` waiting on slice 16's `ApprovalGateComponent` rebuild-from-log behavior and a scripted testy harness that reliably triggers the approval gate. This is the real missing piece of Harness today.
- [ ] Documented contract for what conductor components can do at the suspend/resume seam (closed by slice 16 worked example)

**Status:** Honestly partial. The append/order and load-coordinator halves have live coverage, but the approval-gate-based suspend/resume round trip — the Anthropic contract that makes Harness meaningful for long-running work — has no live invariant yet. Slice 16 closes it.

### Orchestration — acceptance bar

Orchestration is satisfied by composition of existing primitives (see §2 above). The acceptance bar is therefore about **the composition pieces being in place**, not about a new primitive landing.

- [x] `durable-streams` accepts writes from any authenticated producer (not just the runtime)
- [x] `openStream` lets any process subscribe to a runtime's Session log
- [x] `LoadCoordinatorComponent` rebuilds ACP session state from the durable log
- [x] `RuntimeHost::create` cold-starts a runtime against a spec
- [x] Rust-side composition reduction has a live acceptance contract at `tests/managed_agent_primitives_suite.rs:132` (`managed_agent_orchestration_acceptance_contract`)
- [x] `runtimeSpec` is durably persisted as a Session event at provision time so `wake` can read it back — exercised by `reconstruct_runtime_spec_from_log` inside `tests/managed_agent_primitives_suite.rs:132`
- [ ] `wake(sessionId)` helper shipped in `@fireline/client` — not live yet; this is the "not product-ready or TS-owned" half of Orchestration
- [x] Live runtime no-op / concurrent-wake safety are covered at `tests/managed_agent_orchestration.rs:84` (`orchestration_resume_on_live_runtime_is_noop`) and `tests/managed_agent_orchestration.rs:161` (`orchestration_concurrent_resume_creates_single_runtime`)
- [x] At least one worked example of a "waker" subscriber loop, with coordination through the durable stream — `tests/managed_agent_orchestration.rs:249` (`orchestration_subscriber_loop_drives_pause_release_cycle`)
- [ ] At least one consumer proves the full cycle end-to-end: component suspends → event appended → subscriber sees it → calls `wake` → runtime cold-starts if needed → `session/load` rebuilds → component releases the pause → agent advances (closed by slice 16)

**Status:** Strong on the Rust-side composition story. The remaining gap is product/API ownership: `@fireline/client` still does not expose `wake(sessionId)`, and the full "pause survives runtime death" cycle still depends on the pending Harness suspend/resume seam.

### Resources — acceptance bar

The primitive splits into two halves (see §5); the bar covers both, and the coverage now splits into "live in the local-runtime suite" vs. "intentionally delegated to Docker-scoped coverage".

**Physical mounts (for shell-based agents):**

- [x] `resources: Vec<ResourceRef>` field and `ResourceMounter`-shaped physical-mount acceptance at the component layer — `tests/managed_agent_primitives_suite.rs:249` (`managed_agent_resources_physical_mount_acceptance_contract`) passes
- [ ] Shell-visible physical mount proven end-to-end inside a container filesystem — intentionally kept as a Docker-scoped cross-reference marker at `tests/managed_agent_resources.rs:154` (`resources_physical_mount_is_shell_visible_inside_runtime`) and `tests/managed_agent_primitives_suite.rs:290` (`managed_agent_resources_physical_mount_shell_visibility_contract`). Local runtimes have no container filesystem to prove this against; do not promote these stubs.
- [ ] `GitRemoteMounter` implementation (slice 15)
- [ ] Documented contract for how mounters interact with `RuntimeProvider::start()`

**ACP fs interception (for ACP-native file ops and artifact capture):**

- [x] `FileBackend` trait, `FsBackendComponent` in `fireline-components`, and `SessionLogFileBackend` covered at the component layer — `tests/managed_agent_primitives_suite.rs:369` (`managed_agent_resources_fs_backend_component_test`) passes
- [x] End-to-end test where an agent `fs/write_text_file`s through a launched runtime and the event lands on the Session log — `tests/managed_agent_resources.rs:195` (`resources_fs_backend_captures_write_as_durable_event`) and `tests/managed_agent_primitives_suite.rs:308` (`managed_agent_resources_fs_backend_acceptance_contract`)
- [x] Cross-runtime virtual filesystem via the stream-backed backend — `tests/managed_agent_resources.rs:287` (`resources_session_log_backend_supports_cross_runtime_reads`)

**Status:** Strong on ACP-fs interception and component-layer physical mounts. The only intentionally ignored acceptance row is the Docker-scoped shell-visible mount proof, which stays external to the local-runtime managed-agent suite by design. Remaining work is additive depth: more mounters and clearer provider/mounter documentation.

## How to add a new slice

When proposing new work, the slice doc should follow this template:

1. **Which primitive does this slice extend?** Pick exactly one. If it doesn't fit, the slice is the wrong shape — propose it as a Fireline doc only if the gap is in this doc, otherwise it belongs in a downstream product.
2. **Which acceptance-bar items does this slice close?** Reference the checkbox list above. If a slice doesn't close any acceptance-bar item, it's likely premature optimization or product-layer scope.
3. **What does this slice depend on?** Cite by slice number and primitive name. Avoid hidden dependencies.
4. **What does this slice unblock?** Cite the downstream slices and primitives.
5. **Acceptance criteria** in the standard execution-doc shape.
6. **Validation** in the standard execution-doc shape.

This template lives in `docs/execution/SLICE_TEMPLATE.md` (to be created when slice 16 is rewritten under this template, since slice 16 is the first slice to use the new primitive-anchored shape end-to-end).

## What this means for slice doc rewrites

The existing 13a → 17 plan in `docs/execution/` doesn't need a rewrite of its content — only of its framing. Each slice doc gets a header section that says which primitive it extends and which acceptance-bar items it closes, and the body is updated to use the primitive vocabulary. Specific changes per slice:

- **Slice 14 (runs and sessions API)** — reframe header: extends Session, closes the canonical-row-schema and TS-materialization items, **plus the durable `runtimeSpec` persistence** that `wake` relies on. Body: replace "Run and Session as product objects" with "Session as a Fireline read surface that downstream products consume."
- **Slice 15 (workspace object)** — replace entirely with a Resources refactor doc that defines `ResourceRef`, `ResourceMounter`, and the first two mounters.
- **Slice 16 (out-of-band approvals)** — reframe header: first worked example of Orchestration composition (no longer "consumer of slice 18"). Body: upgrade `ApprovalGateComponent` to rebuild from the log on `session/load`, ship a waker subscriber worked example, document multi-subscriber coordination.
- **Slice 17 (capability profiles)** — reframe header: extends Tools. Body: replace "CapabilityProfile product object" with "portable launch input bundling tool refs and credential refs."
- **Slice 18 (deleted)** — the doc was never written; the Orchestration reduction in §2 means no dedicated slice is needed. The work folds into slices 14, 16, and the TS API surface.
- **Slice 13a, 13b, 13c (existing)** — add a one-line header noting they extend Sandbox. Existing doc bodies are correct, just need the framing.

The doc audit running in parallel (`docs/explorations/doc-staleness-audit.md`) produces the concrete delta-by-paragraph list to apply. Note that the audit was written before the Orchestration reduction; its line-by-line deltas for slice 16 should now target "first Orchestration-composition consumer" rather than "consumer of slice 18 `wake`."

## What this means for the substrate exploration

The exploration workstream gets dramatically simpler. Instead of "8–10 capability gaps from two RFC suites," it's **"Fireline's implementation of six managed-agent primitives, with two real gaps (Orchestration and Resources)."** Same vocabulary works for Anthropic, Flamecast, and Rivet.

The exploration deliverables become:

- **D1 — this doc.** The orienting anchor. Cites `managed-agents-citations.md` (codex DAR is producing that in parallel) for file:line evidence.
- **D2 — Proposed TypeScript surface.** Six TypeScript interfaces, one per primitive, with adapter examples showing Flamecast's RFCs implemented on top. The two missing primitives (Orchestration, Resources) define new types; the four strong primitives consolidate what `@fireline/client` and `@fireline/state` already export.
- **D3 — Layer alignment recommendation.** Which `fireline/packages/*` hosts each primitive. The eventual `@fireline/*` npm publishing question.

The exploration is **codex DAR's lane**. They produce D1's citations now, then D2 once this anchor doc is committed, then D3 once D2 is reviewed.

## Open questions

These are deliberately not pinned by this doc — they will be decided when the orchestration slice (18) is drafted or when the resources slice rewrite happens.

1. **Where does the `wake` scheduler live?** In-process inside the control plane (simplest) vs. an external scheduler service (more flexible, more parts to operate). The first cut should be in-process; pulling it out is a later refactor.
2. **What's the minimum harness state that must be persisted across `wake` calls?** ACP session state is part of it. The conductor proxy chain's mid-flight state (e.g., a paused `ApprovalGateComponent`) is another part. There's a clean split here: ACP session state lives in the durable session log; conductor pause state lives in component-specific durable records.
3. **Does `wake` retry on failure, or is the caller responsible?** Anthropic's "any scheduler that can call a function with an ID and retry on failure" implies the scheduler retries. Fireline's first cut should follow that — the scheduler holds the retry policy, callers fire-and-forget.
4. **What's the right granularity for `Resources` mounters?** `LocalPathMounter` is obvious. `GitRemoteMounter` could be one mounter or several (clone, archive, sparse-checkout). Defer this until we have a concrete second mounter to compare against.
5. **Does Fireline ever own the harness loop directly?** Anthropic's framing assumes the substrate calls the harness; Fireline currently delegates to an external agent process. There's a hypothetical "embedded harness" model where Fireline itself runs the loop, but the conductor-proxy approach is more flexible and aligns with ACP's external-agent assumption. Open: revisit if a future product needs Fireline to own the loop.
6. **Is the Sandbox `provision()` / `execute()` split worth surfacing in TypeScript?** Today `client.host.create()` does both atomically. A future world might want to separate them ("provision and keep warm" vs. "execute against an already-provisioned runtime"). Defer until a second consumer wants the split.

## Updates to this doc

This doc is the orienting anchor for the substrate exploration. Edit it (don't replace it) when:

- A primitive's status changes (e.g., Orchestration moves from Missing → Partial → Strong as slice 18 ships)
- A new gap is discovered against the framework
- The slice plan changes in a way that affects the primitive mapping

Successor docs (TypeScript surface proposal, layer recommendation) cite this doc by section heading. The six-primitive vocabulary is now Fireline's canonical language for substrate discussions.
