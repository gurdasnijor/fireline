# Fireline against Anthropic's Managed-Agent Primitives

> Status: **operational source of truth** for Fireline's substrate roadmap
> Type: reference + decision + execution-driving doc
> Audience: maintainers deciding what to build, in what order, against what acceptance bars
> Source: Anthropic engineering blog, *"Managed agents: a small set of primitives for any agent harness"* (https://www.anthropic.com/engineering/managed-agents)
> Related:
> - [`../product/priorities.md`](../product/priorities.md) — substrate-first product positioning (derives from this doc)
> - [`../runtime/control-and-data-plane.md`](../runtime/control-and-data-plane.md) — the two-plane architecture this doc maps onto
> - [`../runtime/heartbeat-and-registration.md`](../runtime/heartbeat-and-registration.md) — the push lifecycle that Sandbox / Orchestration depend on
> - [`../execution/README.md`](../execution/README.md) — slice index, organized by which primitive each slice extends
> - [`./managed-agents-citations.md`](./managed-agents-citations.md) — the file:line inventory this doc cites against

## How to read this doc

This is the **source of truth** for what Fireline should build, in what order, and against what acceptance bars. Three other docs derive from it:

- `docs/product/priorities.md` derives the "what we own / what we don't" framing and the high-level slice ordering
- `docs/execution/README.md` derives the slice index, with each slice tagged by which primitive it extends
- Each individual slice doc in `docs/execution/` opens with which primitive it implements and which gap it closes

If you're picking up new work, start here. If you're writing a new slice doc, cite this doc by section heading and pick a primitive to anchor against. If a slice doesn't fit any primitive, that's a signal the slice is the wrong shape — it may belong in a downstream product, not in Fireline.

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
| 1 | **Session** | `getSession(id) → (Session, Event[])`; `getEvents(id) → PendingEvent[]`; `emitEvent(id, event)` | Any append-only log consumed in order from any event point with idempotent appends | **Strong** |
| 2 | **Orchestration** | `wake(session_id) → void` | Any scheduler that can call a function with an ID and retry on failure | **Missing** |
| 3 | **Harness** | `yield Effect<T> → EffectResult<T>` | Any loop that yields effects and appends progress to the Session | **Partial** (by design) |
| 4 | **Sandbox** | `provision({resources}) → execute(name, input) → String` | Any executor configured once and called many times as a tool | **Strong** |
| 5 | **Resources** | `[{source_ref, mount_path}]` | Any object store the container can fetch from by reference | **Missing** |
| 6 | **Tools** | `{name, description, input_schema}` | Any capability describable as a name and input shape | **Strong** |

**One-line summary:** Fireline already has Session, Sandbox, and Tools strong; Harness is partial-by-design; Orchestration and Resources are the two real gaps and they are tightly coupled.

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

| Fireline feature | Anthropic primitive | Combinator |
|---|---|---|
| `client.topology.attach('audit', ...)` | Session + Harness | `appendToSession . mapEffect` |
| `client.topology.attach('approval-gate', ...)` | Session + Orchestration + Harness | `suspend` |
| `client.topology.attachTool({...})` | Tools + Harness | `mapEffect` over init |
| `client.host.create({ resources: [...] })` | Resources + Harness | `provision` (init-time Component) |
| `client.host.create({ topology })` | Sandbox + Harness | `compose` of all topology components |
| `client.state.session.get(id)` | Session | materializer fold |
| `client.stream.replay(endpoint, cursor)` | Session | identity fold (raw passthrough) |
| `client.orchestration.wake(key, reason)` | Orchestration | `wake` (the primitive itself) |
| `client.acp.session.prompt(...)` | Sandbox.execute + Harness yield | direct passthrough |

Everything Fireline does is reducible to "primitive(s) + combinator". This is the operational answer to "what belongs in Fireline vs. what belongs in Flamecast." Fireline is the substrate that provides the six primitives plus the seven-combinator algebra. Flamecast composes those into product objects (runs, workspaces, profiles, approval queues) that don't fit the combinator algebra and shouldn't.

### Why this matters operationally

When proposing a new Fireline feature, run it through this test:

1. **Express it as a combinator first.** If it's a new conductor component, decompose it into the seven combinators. If it's a materializer, write it as a `(Event, S) → S` fold step.
2. **If you cannot decompose it, be suspicious.** A feature that doesn't fit the combinator algebra is reaching for something the substrate doesn't have. Either Fireline needs a new primitive (unusual — only Orchestration and Resources have qualified so far) or the feature is a product object that belongs in Flamecast.
3. **If it composes cleanly, you are probably right.** The combinator algebra is the test for "does this fit Fireline's shape" before any code is written.

This is also the test for whether a feature is *too small* to belong in Fireline. If a proposed component is "just `mapEffect(addHeader)`", that's a one-liner; it's a configuration, not a component. Fireline ships components that have meaningful internal state or non-trivial composition; one-liners belong in user code.

## 1. Session — Strong

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

None at the substrate level. The persistence tier exists, the replay protocol exists, the producer and at least two consumer kinds exist, the architectural role is committed.

The remaining work is **stabilizing the row schema** so that downstream products can read the same Session events without forking — that's slice 14 in the rewritten priorities, framed as "a stable read contract over durable state" rather than "session as a product object."

### How existing slices contribute

- **Already shipped:** durable-streams integration, `DurableStreamTracer`, runtime-local materializers, TS `StreamDB`
- **Slice 14 (in plan)** — canonical row schema, replay/catch-up TypeScript surface, distinction between hot ACP traffic and cold read-oriented state
- **Slice 13b (just shipped)** — added the runtime descriptor and registration shape that Session events reference

## 2. Orchestration — Missing (biggest gap)

### Anthropic interface

```text
wake(session_id) → void
```

Satisfied by *"any scheduler that can call a function with an ID and retry on failure — a cron job, a queue consumer, a while-loop, etc."*

The interface is one method. The implication is enormous: **the harness loop is a function the scheduler calls, not a process the user starts.** If `wake` is the entry point, the harness can be dormant between calls, the runtime can be torn down and rebuilt, the session state survives in the durable log, and every "advance the agent" trigger — webhook arrived, timer fired, approval granted, peer call returned — funnels through the same primitive.

### What Fireline exposes

Today, **nothing**. There is no scheduler, no `wake` function, no notion that a runtime is dormant. The model is "the runtime is a long-lived process that holds the ACP session open until the user disconnects." If the runtime dies, the session dies with it.

The closest analog is the control plane creating a runtime and the runtime self-registering, but that's "boot the harness once and keep it alive," not "advance the harness one step on demand from a stored state."

### What this unlocks

Adding `wake` is the **background-agent primitive**. It's the difference between "agent runs in a tab" and "agent keeps working while you sleep, then notifies you on Slack when it needs approval, then resumes on its own when approval arrives." Specifically, `wake` is the load-bearing dependency for:

- **Out-of-band approvals** (slice 16). Approval lands → control plane calls `wake(runtime_key)` → the runtime resumes from its durable state, sees the approval, advances one step.
- **Webhook ingestion** (Flamecast unified-runtime + Rivet webhooks pattern). External event arrives at an HTTP endpoint → an ingress component enqueues into the durable log → calls `wake` → the runtime drains and processes.
- **Queue management** (Flamecast queue-management RFC). Each enqueued prompt is a `wake` trigger; pause/resume become "stop calling wake / start calling wake again."
- **Multiplayer** (Rivet multiplayer pattern). New observer connects → reads `getSequencedEvents` from the durable log → no `wake` needed. New driver sends a prompt → `wake(runtime_key)` advances the agent.
- **Cross-runtime peer calls** (Spike 5 lineage, slice 13c cross-runtime proof). Peer A calls peer B → arrives as a tool call → on the receiving side, `wake(runtime_key)` ensures the target runtime is alive to handle it.

### What it would take

The implementation has three layers:

1. **A scheduler service** that owns the `wake(runtime_key)` entry point. For a first cut this can be an in-process tokio task inside the control plane that maintains a queue of `(runtime_key, reason)` pairs and calls `RuntimeProvider::start()` on demand when no live runtime exists for that key. Retries on failure are the standard backoff loop.
2. **Runtime side**: the runtime, after registering, must be able to **catch up to its durable state on start** rather than starting empty. The runtime-local materializer pattern already does this for session indexes; the same pattern needs to apply to the harness's "where am I in this conversation" pointer. This is partially served by ACP's session persistence but needs a Fireline contract for "given runtime_key, restore the harness to where it left off."
3. **External triggers**: webhooks, timers, peer calls, approval responses all need to terminate at a `POST /v1/runtimes/{key}/wake` endpoint on the control plane (or equivalent in-process call). This is a thin wrapper around the scheduler.

### Cost estimate

A first-cut implementation is **one execution slice** — call it slice 18, "Orchestration and the wake primitive." It depends on slice 13c (Docker provider) being able to spin a fresh runtime against a stored session, which it already needs to do for cold-start anyway.

### How existing slices contribute

- **Already shipped:** runtime registration + heartbeat (the runtime can be re-instantiated and re-attach to a `runtime_key`), durable session events (state survives the runtime)
- **Slice 13c (in flight)** — proves cold-start works for non-local providers, which is what `wake` exercises
- **Slice 18 (proposed, new)** — the actual `wake` primitive
- **Slice 16 (in plan, reframed)** — out-of-band approvals become a *consumer* of `wake`, not a separate orchestration mechanism

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

## 5. Resources — Missing

### Anthropic interface

```text
[{source_ref, mount_path}]
```

Satisfied by *"any object store the container can fetch from by reference — Filestore, GCS, a git remote, S3."*

### What Fireline exposes

The closest analog is the helper file API (`/api/v1/files/{...}` on the runtime) which lets the agent read files from the host filesystem. But that's local-file-only and assumes the host filesystem is the resource source. There is no first-class concept of "this run depends on these external resource references; mount them at these paths before the agent starts."

### Why this matters

Fireline's existing slice 15 (`docs/execution/15-workspace-object.md`) tried to solve this by introducing a heavy "Workspace product object" with identity, lifecycle, and product semantics. **Anthropic's framing is dramatically simpler:** it's just `[{source_ref, mount_path}]` — a list of refs paired with where they should land in the runtime.

This collapses slice 15 from a product-object problem into a launch-spec field. The implementation:

- **`CreateRuntimeSpec`** grows a `resources: Vec<ResourceRef>` field where `ResourceRef = { source_ref: String, mount_path: PathBuf }`
- **A `ResourceMounter` trait** with implementations:
  - `LocalPathMounter` — bind-mounts a local directory (the current de facto behavior)
  - `GitRemoteMounter` — clones a repo into the mount path
  - `S3Mounter` — fetches an S3 prefix into the mount path
  - `GcsMounter` — same for Google Cloud Storage
- **`RuntimeProvider::start()`** invokes the appropriate mounter for each `ResourceRef` before launching the agent

This is **a week of work, not a slice**. The current slice 15 doc should be heavily revised or demoted.

### How existing slices contribute

- **Slice 15 (currently in plan as "workspace object")** — needs to be reframed as the Resources primitive: a launch-spec field plus pluggable mounters, not a product object
- **Slice 13c (in flight)** — Docker provider needs to mount *something* into the container, so a minimal `LocalPathMounter` will probably land here as a side effect
- **Future slices:** richer mounters (S3, GCS, git) added incrementally

## 6. Tools — Strong

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

None. Tools are arguably Fireline's most flexible primitive — the topology component model is more expressive than a flat tool list because it lets components compose, transform, and observe each other's tool registrations.

The remaining work is **portable references** — letting a run carry "I want these tools mounted, fetched from these credential sources" rather than baking the tool list into the spawn arguments. That's slice 17 (capability profiles), reframed as:

- A capability profile is a list of `{name, description, input_schema, transport_ref, credential_ref}` entries
- `transport_ref` points to "where to fetch this tool" (Smithery URL, peer runtime key, MCP server endpoint)
- `credential_ref` points to "where to resolve auth at call time" (secret store path, environment binding, per-session OAuth token)

This keeps credentials out of the runtime and out of the spawn spec.

### How existing slices contribute

- **Already shipped:** `PeerComponent`, `SmitheryComponent`, topology MCP injection, conductor proxy chains
- **Slice 17 (in plan, reframed)** — capability profiles as portable Tools references with credential_ref indirection
- **External auth seam** (in priorities #5) — the credential_ref resolution layer

## The two real gaps, in priority order

### Gap 1: Orchestration (`wake(session_id)`)

This is the largest single missing primitive in Fireline today. It is the dependency for:

- Background agents that survive runtime death
- Out-of-band approvals (slice 16)
- Webhook ingestion (Flamecast and Rivet patterns)
- Queue management with pause/resume (Flamecast queue RFC)
- Multiplayer driver flows (Rivet multiplayer pattern)
- Cross-runtime peer calls that target dormant runtimes
- Suspend/resume of the Harness primitive

It's the difference between **Fireline as "managed agent runtime hosting"** and **Fireline as "managed agent substrate that products like Flamecast can build durable workflows on top of."**

**Recommended action:** add as slice 18, "Orchestration and the wake primitive." Sequenced after slice 13c (Docker provider) and slice 14 (canonical session read schema), since both feed into what `wake` needs to be able to do.

### Gap 2: Resources (`[{source_ref, mount_path}]`)

The smaller gap, but the one that simplifies the existing slice plan dramatically. The current slice 15 ("workspace object") tries to solve a hard product-object problem; the Anthropic framing collapses it to a launch-spec field with pluggable mounters.

**Recommended action:** demote slice 15 from "execution slice" to "small refactor." Replace `docs/execution/15-workspace-object.md` with a much shorter doc that defines `ResourceRef`, `ResourceMounter`, and the four initial implementations (LocalPath, GitRemote, S3, Gcs). Estimated cost: a week, not a slice.

## Build order and slice index

This is the operational plan: which slices ship in what order to close the gaps and harden the strong primitives. Each slice is tagged by which primitive it extends.

### Slice index, organized by primitive

| Primitive | Status | Slices that contribute | Status of those slices |
|---|---|---|---|
| **Session** | Strong | `14` Session as canonical read surface | Doc planned, implementation not started |
| **Sandbox** | Strong | `13a` control-plane runtime API; `13b` push lifecycle and auth; `13c` first remote provider (Docker via bollard) | 13a + 13b shipped on `main`; 13c in flight in workspace 7 |
| **Tools** | Strong | `17` capability profiles as portable tool references | Doc planned, will be reframed from heavy product object to portable refs with `credential_ref` indirection |
| **Harness** | Partial (by design) | None standalone; depends on `18` | Conductor proxy chain serves the I/O seam today; suspend/resume across runtime death is gated on `18` |
| **Orchestration** | **Missing** | `18` orchestration and the `wake` primitive (NEW); `16` out-of-band approvals (consumer of `wake`) | `18` to be drafted; `16` to be reframed as a `wake` consumer |
| **Resources** | **Missing** | `15` workspace object → demoted to "Resources primitive: launch-spec field with pluggable mounters" | Existing slice doc needs to be replaced with a much shorter resources refactor doc |

### Build order, with rationales

The order is chosen to maximize unblocking — every slice enables at least one downstream slice or primitive completion.

**1. `13c` Docker provider (Sandbox depth)** — *in flight, workspace 7*

First non-local provider. Forces the push lifecycle from 13b to be exercised end-to-end against a real container. Establishes the `RuntimeProvider` trait as the universal Sandbox boundary. May ship a minimal `LocalPathMounter` as a side effect (preview of slice 15).

**2. `14` Session as canonical read surface** — *next, can start in parallel with 13c*

Stabilizes the row schema downstream products read from the durable state stream: `runtime`, `session`, `prompt_turn`, `permission`, `terminal`, `chunks`, child-session edges. Ships the TypeScript materialization layer that downstream consumers embed. Does not depend on 13c — they're orthogonal lanes.

**3. `15` Resources refactor** — *small, ~1 week, can run in parallel with 13c and 14*

Rewrite slice 15 from "Workspace object" to "Resources primitive." Adds `resources: Vec<ResourceRef>` to `CreateRuntimeSpec`, defines `ResourceMounter` trait, ships `LocalPathMounter` and `GitRemoteMounter` as the first two implementations. S3 and GCS mounters land later.

**4. `18` Orchestration and the `wake` primitive** — *the unblocker*

The biggest single addition. Introduces `POST /v1/runtimes/{key}/wake` on the control plane plus an in-process scheduler that calls `RuntimeProvider::start()` against a stored session when no live runtime exists for the key. Includes the runtime-side "catch up to durable state on start" contract. Depends on `14` (the canonical read schema is what `wake` reads to restore state) and is best built against `13c` (so the cold-start path is exercised against a non-local provider).

This is the load-bearing primitive. Before it ships, Fireline is "managed runtime hosting." After it ships, Fireline is "managed agent substrate."

**5. `16` Out-of-band approvals** — *consumer of `wake`*

Reframed from "approval product object" to "approval gate component that durably persists pending state and triggers `wake` when resolved." The substrate work is small once `18` is in place — a durable `pending_approval` row, a `POST /v1/approvals/{id}/resolve` endpoint that calls `wake(runtime_key)`, and the `ApprovalGateComponent` is upgraded to read durable state on resume.

**6. `17` Capability profiles as portable Tools references** — *Tools depth*

Reframed from heavy "CapabilityProfile product object" to "portable launch input that bundles tool refs + credential refs + topology defaults." Shipped as a launch-spec field, similar to `15`'s collapse. Adds `credential_ref` indirection so credentials resolve at call time rather than spawn time.

**7. Component depth (ongoing)** — *Tools and Harness composition*

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
- [x] Idempotent appends guaranteed by the producer protocol
- [x] At least one runtime-local consumer (`SessionIndex`, `RuntimeMaterializer`)
- [x] At least one external consumer (`packages/state` `StreamDB`)
- [ ] Canonical row schema documented and stable (slice 14)
- [ ] TypeScript materialization layer with replay/catch-up semantics that downstream products can embed (slice 14)
- [ ] Distinction between hot ACP traffic and cold read-oriented state called out in TS surface (slice 14)

**Status:** ~70% complete. Slice 14 closes the remaining items.

### Sandbox — acceptance bar

- [x] `RuntimeProvider` trait with `start()` returning a `RuntimeLaunch`
- [x] `LocalProvider` ships and works in dev mode
- [x] `ChildProcessRuntimeLauncher` ships as the control-plane-backed local provider
- [x] Push lifecycle (`/register`, `/heartbeat`) so providers don't need shared filesystem (slice 13b)
- [x] Bearer auth on push surface, scoped per `runtime_key` (slice 13b)
- [ ] At least one non-local provider (slice 13c — Docker via bollard)
- [ ] Mixed local + non-local topology proof (slice 13c)
- [ ] Cross-runtime peer call traverses mixed topology with reconstructible lineage (slice 13c)

**Status:** ~80% complete. Slice 13c closes the remaining items. Additional providers (E2B, Daytona, Cloudflare) are additive depth and don't gate completion.

### Tools — acceptance bar

- [x] Tools are described by `{name, description, input_schema}` (MCP-shape) at the conductor level
- [x] Topology component model lets tools be injected per session (`PeerComponent`, `SmitheryComponent`)
- [x] Conductor proxy chain lets host code intercept and wrap tool calls
- [ ] Portable tool references in launch spec, with `credential_ref` indirection (slice 17)
- [ ] Credential resolution at call time, not spawn time (slice 17)

**Status:** ~70% complete. Slice 17 closes the remaining items.

### Harness — acceptance bar

- [x] Conductor proxy chain intercepts the harness's I/O (`PromptResponderProxy`, `PromptObserverProxy`, message observers)
- [x] Topology composition lets components wrap, transform, observe, or substitute effects
- [x] All harness progress is appended to the durable session log via `DurableStreamTracer`
- [ ] Conductor components can pause mid-effect and resume from durable state across runtime death (depends on slice 18)
- [ ] Documented contract for what conductor components can do at the suspend/resume seam (depends on slice 18)

**Status:** ~60% complete. Slice 18 unlocks the remaining items because suspend/resume has nowhere to land without `wake`.

### Orchestration — acceptance bar

- [ ] `wake(runtime_key, reason)` primitive on the control plane (HTTP + in-process)
- [ ] In-process scheduler that calls `RuntimeProvider::start()` against a stored session when no live runtime exists for the key
- [ ] Retry-on-failure semantics with exponential backoff
- [ ] Runtime-side contract for "catch up to durable state on start"
- [ ] Documented external triggers: webhook ingress, approval resolution, peer call delivery, timer wake-ups
- [ ] At least one consumer (slice 16 out-of-band approvals) integrated end-to-end

**Status:** 0% complete. Entirely owned by slice 18 plus slice 16 as the first consumer.

### Resources — acceptance bar

- [ ] `resources: Vec<ResourceRef>` field on `CreateRuntimeSpec`
- [ ] `ResourceMounter` trait
- [ ] `LocalPathMounter` implementation (probably from slice 13c side effect)
- [ ] At least one network-fetched mounter — `GitRemoteMounter` or `S3Mounter` (slice 15 rewrite)
- [ ] Documented contract for how mounters interact with `RuntimeProvider::start()`
- [ ] One end-to-end test where a runtime mounts a non-local resource and the agent reads from it

**Status:** 0% complete. Owned by slice 15 rewrite, with a possible head start from slice 13c.

## How to add a new slice

When proposing new work, the slice doc should follow this template:

1. **Which primitive does this slice extend?** Pick exactly one. If it doesn't fit, the slice is the wrong shape — propose it as a Fireline doc only if the gap is in this doc, otherwise it belongs in a downstream product.
2. **Which acceptance-bar items does this slice close?** Reference the checkbox list above. If a slice doesn't close any acceptance-bar item, it's likely premature optimization or product-layer scope.
3. **What does this slice depend on?** Cite by slice number and primitive name. Avoid hidden dependencies.
4. **What does this slice unblock?** Cite the downstream slices and primitives.
5. **Acceptance criteria** in the standard execution-doc shape.
6. **Validation** in the standard execution-doc shape.

This template lives in `docs/execution/SLICE_TEMPLATE.md` (to be created when slice 18 is drafted, since slice 18 is the first slice written under this template).

## What this means for slice doc rewrites

The existing 13a → 17 plan in `docs/execution/` doesn't need a rewrite of its content — only of its framing. Each slice doc gets a header section that says which primitive it extends and which acceptance-bar items it closes, and the body is updated to use the primitive vocabulary. Specific changes per slice:

- **Slice 14 (runs and sessions API)** — reframe header: extends Session, closes the canonical-row-schema and TS-materialization items. Body: replace "Run and Session as product objects" with "Session as a Fireline read surface that downstream products consume."
- **Slice 15 (workspace object)** — replace entirely with a Resources refactor doc that defines `ResourceRef`, `ResourceMounter`, and the first two mounters.
- **Slice 16 (out-of-band approvals)** — reframe header: consumer of Orchestration, depends on slice 18. Body: replace "approval product object" with "durable approval state + `wake` trigger."
- **Slice 17 (capability profiles)** — reframe header: extends Tools. Body: replace "CapabilityProfile product object" with "portable launch input bundling tool refs and credential refs."
- **Slice 18 (NEW: orchestration and wake)** — write from scratch using the slice template above.
- **Slice 13a, 13b, 13c (existing)** — add a one-line header noting they extend Sandbox. Existing doc bodies are correct, just need the framing.

The doc audit running in parallel (`docs/explorations/doc-staleness-audit.md`) produces the concrete delta-by-paragraph list to apply.

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
