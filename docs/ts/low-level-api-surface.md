# Fireline Low-Level TypeScript API Surface

> This doc is the **TypeScript-side companion** to [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md). The mapping doc defines what Fireline builds against Anthropic's six managed-agent primitives; this doc defines how those primitives are exposed to TypeScript consumers.
>
> Related:
> - [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md) — operational source of truth for the six primitives
> - [`../explorations/managed-agents-citations.md`](../explorations/managed-agents-citations.md) — file:line evidence for each primitive's current Rust implementation
> - [`primitives.md`](./primitives.md)
> - [`../product/priorities.md`](../product/priorities.md)
> - [`../runtime/control-and-data-plane.md`](../runtime/control-and-data-plane.md)
> - [`../state/session-load.md`](../state/session-load.md)
> - [ACP proxy chains](https://agentclientprotocol.com/rfds/proxy-chains)

## Purpose

This doc defines the low-level TypeScript API surface Fireline should expose before any ergonomic or product-layer wrappers.

The goal is to answer:

- what are the primary nouns?
- which namespace owns each noun?
- which operations are reads vs mutations?
- which surfaces belong to the control plane vs the data plane?
- which surfaces are stable portability seams across local and remote runtimes?
- **which Anthropic managed-agent primitive does each namespace anchor?**

This is a systems contract, not a product SDK.

## Primitive Anchoring

Fireline's TypeScript surface is anchored against the six managed-agent primitives from Anthropic's [*"Managed agents"* post](https://www.anthropic.com/engineering/managed-agents). Every namespace and noun in this doc maps to one or two of these primitives — if a proposed surface doesn't fit, it belongs in a higher product layer, not in the low-level API.

| # | Primitive | Anthropic interface | Fireline TS namespace(s) | Fireline TS noun(s) |
|---|---|---|---|---|
| 1 | **Session** | `getSession(id) → (Session, Event[])`; `getEvents(id) → PendingEvent[]`; `emitEvent(id, event)` | `client.state` (read) + `client.stream` (raw replay) | `SessionDescriptor`, `StreamEndpoint`, materialized rows |
| 2 | **Orchestration** | `wake(session_id) → void` | no dedicated namespace; satisfied by composition across `client.state`, `client.host`, and `client.acp` | `resume(sessionId)` helper, subscriber loop |
| 3 | **Harness** | `yield Effect<T> → EffectResult<T>` | `client.topology` (composition surface) + `client.acp` (effect transport) | `TopologySpec`, conductor components |
| 4 | **Sandbox** | `provision({resources}) → execute(name, input) → String` | `client.host` (provision/lifecycle) + `client.acp` (execute channel) | `RuntimeDescriptor`, `Endpoint` |
| 5 | **Resources** | `[{source_ref, mount_path}]` | spec field on `client.host.create` (no top-level namespace) | `ResourceRef` |
| 6 | **Tools** | `{name, description, input_schema}` | `client.topology` (tool registration) | `ToolSpec` (a kind of conductor component), `CapabilityRef` for portable refs |

Key implications:

- **Sandbox is split across two namespaces.** `provision()` is `client.host.create()` because the runtime is a long-lived ACP server, not a single `execute()` call. `execute(name, input)` is what `client.acp` does — every ACP request is one Sandbox execution against a long-lived Sandbox instance.
- **Session is split across two namespaces.** `client.state` is the materialized read side that downstream products use; `client.stream` is the raw replayable durable log that consumers can subscribe to directly. Together they cover `getSession`, `getEvents`, and durable replay.
- **Tools and Harness share `client.topology`.** Conductor components are the implementation of both: components ARE the proxy chain that intercepts the harness's effects (Harness primitive), and tools are a kind of component that injects an MCP-shaped capability into the chain (Tools primitive). See [§Conductor and Proxy Chain](#conductor-and-proxy-chain) below.
- **Resources doesn't get its own top-level namespace.** It's a launch-spec field — `resources: ResourceRef[]` on `client.host.create()`. Top-level namespace would be over-engineering for what is essentially `[{source_ref, mount_path}]` plus pluggable mounters on the runtime side.
- **Orchestration does not get its own namespace.** It is satisfied by composition — Session reads, provision/cold-start, ACP reconnect, and `session/load` — with `resume(sessionId)` as the helper surface. See [`managed-agents-mapping.md`](../explorations/managed-agents-mapping.md) §2.

## Conductor and Proxy Chain

The conductor and its proxy chain are how Fireline implements the **Harness** primitive (and the **Tools** primitive that rides on top of it).

### How Fireline differs from Anthropic's framing

Anthropic's `Harness` interface is `yield Effect<T> → EffectResult<T>`: a loop that yields effects and gets back results. The implicit assumption is that the substrate calls the harness — the substrate is the loop runner, the harness is the loop body.

Fireline doesn't call the harness. The **harness is the agent process** — Claude Code, Codex, fireline-testy, or any ACP-speaking subprocess. The agent owns its own loop. Fireline sits *between* the harness and the outside world via the conductor proxy chain.

This is a deliberate choice. It's more flexible than Anthropic's framing because:

- multiple components can compose around a single effect
- components can pause mid-effect (an `ApprovalGateComponent` holds a tool call until approval lands)
- components can fan out to peers (a `PeerComponent` turns one tool call into multiple sub-effects on remote runtimes)
- components can persist progress to the durable Session log without the harness knowing (`DurableStreamTracer`)
- the ACP protocol is the universal contract — anything that speaks ACP can be a harness

### How the proxy chain maps onto Harness

Concretely: every effect the harness yields is an ACP request from the agent — `session/prompt`, `tools/call`, `mcp/list_resources`, etc. The proxy chain is a series of components that wrap these requests on the way out and the way back:

```text
agent process (harness)
       │
       │  yield Effect (ACP request)
       ▼
┌────────────────────────────────────┐
│  Conductor proxy chain             │
│                                    │
│  ┌──────────────────────────────┐  │
│  │ AuditTracer                  │  │  ← observes
│  ├──────────────────────────────┤  │
│  │ ContextInjectionComponent    │  │  ← transforms
│  ├──────────────────────────────┤  │
│  │ ApprovalGateComponent        │  │  ← suspends + waits
│  ├──────────────────────────────┤  │
│  │ BudgetComponent              │  │  ← can block
│  ├──────────────────────────────┤  │
│  │ PeerComponent / SmitheryComp │  │  ← injects tools, fans out
│  ├──────────────────────────────┤  │
│  │ DurableStreamTracer          │  │  ← persists to Session
│  └──────────────────────────────┘  │
└────────────────┬───────────────────┘
                 │
                 │  effect lands at
                 │  destination
                 ▼
       (LLM, tool, MCP server, peer)
                 │
                 │  EffectResult flows back
                 │  through the chain in reverse
                 ▼
            agent process
```

Each component can:

- **observe** the effect (AuditTracer)
- **transform** the effect (ContextInjectionComponent injects extra context into a prompt)
- **substitute** the effect (PeerComponent rewrites a `tools/call` to a peer ACP call)
- **suspend** the effect (ApprovalGateComponent holds the response until approval lands)
- **block** the effect (BudgetComponent rejects calls over budget)
- **persist** the effect (DurableStreamTracer writes every step to the durable Session log)

This composition is what gives Fireline the Harness primitive's flexibility, plus the suspend/resume capability that becomes meaningful once the `resume(sessionId)` composition is wired up.

### How Tools layers on top

Tools are a *kind* of conductor component. When you attach a Tool to a topology, what you're really doing is registering an MCP-shaped capability that the proxy chain exposes to the agent.

The Tools primitive interface (`{name, description, input_schema}`) is satisfied by:

- `PeerComponent` registering `list_peers` and `prompt_peer` as MCP tools
- `SmitheryComponent` registering arbitrary tools from the Smithery catalog
- Custom user-written components that implement the same shape

From the agent's perspective, all tools look like MCP tools — the conductor handles the routing inside the proxy chain.

### TS-side seam: `client.topology`

The topology is the public face of the proxy chain. Building a topology is how you compose conductor components and tools at the TS layer:

```ts
const topology = client.topology
  .builder()
  .attach('audit', { sink: 'durable-stream' })
  .attach('context-injection', { sources: [{ kind: 'workspace_files' }] })
  .attach('approval-gate', { policy: 'manual', timeout_ms: 60_000 })
  .attach('budget', { tokens: 1_000_000 })
  .attach('peer', { peers: ['runtime:reviewer', 'runtime:writer'] })
  .build()

const runtime = await client.host.create({
  agent: { command: 'codex' },
  topology,
  // ...
})
```

The topology spec is serialized into the runtime's launch arguments, the runtime constructs its proxy chain at startup, and from that moment on every effect the agent yields flows through the components in order.

For tool-flavored components, there's a sugar method:

```ts
client.topology.builder()
  .attachTool({
    name: 'review_pr',
    description: 'Review a GitHub pull request',
    input_schema: {
      type: 'object',
      properties: { url: { type: 'string' } },
      required: ['url'],
    },
    transport_ref: { kind: 'mcp_url', url: 'https://...' },
    credential_ref: { kind: 'env', var: 'GITHUB_TOKEN' },
  })
  .build()
```

`attachTool` is a thin wrapper that registers a one-tool component with the right schema. Bulk tool registration goes through component attach (e.g., a Smithery component registers many tools at once).

### Why proxy/extension is NOT a public noun

The architectural mechanism is ACP proxy composition. The public TypeScript noun is `topology`. This is intentional — see [Design Constraint 5](#5-acp-proxies-are-implementation-topology-is-public-api). Three reasons:

1. **`topology` is closer to the runtime contract.** The runtime accepts a `TopologySpec` at launch; the proxy chain is constructed from it. There's no second config system.
2. **`topology` is less coupled to Rust internals.** Proxy chain construction is a Rust implementation detail. Topology is the wire shape.
3. **`topology` is less confusing.** Users who don't think in proxy terms can still attach components and tools by name.

A future Fireline version could swap out the proxy chain for a different harness composition mechanism without changing the topology API. That portability is worth the slightly less direct vocabulary.

## Design Constraints

### 1. Noun-first, not workflow-first

The API should be organized around a small number of substrate nouns:

- runtime
- session
- endpoint
- stream
- topology
- workspace ref
- capability ref
- approval request, later if durable waits become real

It should not start from workflows like:

- "start coding agent"
- "run cloud worker"
- "install extension"

Those can be composed later.

### 2. One noun, one config surface

The API should avoid parallel abstractions for the same thing.

Examples:

- ACP proxy-chain configuration lives in `topology`
- runtime placement and lifecycle live in `host`
- session mutation lives in `acp`
- durable read surfaces live in `state` or `stream`

Do not introduce a second public noun like `extensions` when the actual
low-level primitive is `topology`.

### 3. Control plane and data plane must remain visible

The API should make this split explicit:

- control plane: runtime lifecycle, runtime discovery, topology metadata,
  portable refs
- data plane: ACP traffic, durable streams, helper file reads

The low-level API should not hide data-plane traffic behind a control-plane
wrapper.

### 4. Portability should flow through descriptors

The portability seam is not a magic session wrapper.

The portability seam is:

- a runtime descriptor with advertised endpoints
- portable refs for workspace and capabilities
- a topology spec that can be attached to a runtime

### 5. ACP proxies are implementation, topology is public API

The architectural secret sauce is ACP proxy composition.

But the public low-level noun should still be `topology`, not `proxy` and not
`extension`.

That keeps the API:

- closer to the runtime contract
- less coupled to Rust internals
- less confusing for users who do not need to think in proxy implementation
  terms

## Top-Level Namespaces

The low-level TypeScript layer is constrained to five namespaces, each anchored
on a managed-agent primitive, plus a small number of composition helpers such
as `resume(sessionId)`:

```ts
client.host           // Sandbox primitive — provision/list/stop runtimes
client.acp            // Sandbox.execute + Harness I/O — ACP wire protocol
client.state          // Session primitive (read side) — materialized rows
client.stream         // Session primitive (raw) — replayable durable stream
client.topology       // Harness + Tools primitives — conductor composition
```

There is intentionally **no** `client.orchestration` namespace. Orchestration is
satisfied by composition — read Session state, provision if dormant, reconnect
ACP, and `loadSession(sessionId)` — with `resume(sessionId)` as the helper.
See [`managed-agents-mapping.md`](../explorations/managed-agents-mapping.md)
§2.

`client.approvals` is intentionally not a top-level namespace. Out-of-band
approvals are a consumer of Session events plus the `resume(sessionId)` helper
— they don't need their own namespace at the low level. A future ergonomic
wrapper at a higher layer may expose `client.approvals.*`, but the primitive
contract lives in Session evidence plus composition.

Intentionally not low-level namespaces, even later:

```ts
client.extensions
client.workloads
client.runs
client.profiles
client.workspaces
```

Those may exist later at a higher product layer, but they should not define the primitive contract. Each one composes multiple lower-level nouns and would blur ownership boundaries if introduced before the substrate is sharper.

## Primary Nouns

## `Endpoint`

The most basic portable noun is an endpoint.

```ts
type Endpoint = {
  url: string
  headers?: Record<string, string>
}
```

Rules:

- endpoints are fully advertised by the producer surface that owns them
- auth headers or bearer tokens travel with the endpoint
- `client.acp` and `client.state` consume endpoints; they do not discover them

Sub-kinds:

- `AcpEndpoint`
- `StreamEndpoint`
- `HelperApiEndpoint`

These can all share the same base shape.

## `RuntimeDescriptor`

This is the primary control-plane noun.

```ts
type RuntimeDescriptor = {
  runtimeKey: string
  runtimeId: string
  nodeId: string
  provider: string
  providerInstanceId: string
  status:
    | "starting"
    | "ready"
    | "busy"
    | "idle"
    | "stale"
    | "broken"
    | "stopped"
  acp: Endpoint
  state: Endpoint
  helperApiBaseUrl?: string
  createdAtMs: number
  updatedAtMs: number
}
```

Ownership:

- owned by `client.host`
- returned by the control plane or local host provider

What it means:

- a runtime descriptor is the canonical answer to "where is this runtime and
  how do I talk to it?"
- it is the bridge between lifecycle/discovery and hot-path data-plane usage

Read operations:

```ts
client.host.get(runtimeKey)
client.host.list()
```

Mutation operations:

```ts
client.host.create(spec)
client.host.stop(runtimeKey)
client.host.delete(runtimeKey)
```

Portability guarantee:

- if a runtime is reachable, every client should be able to consume the same
  advertised `acp` and `state` endpoints regardless of whether the runtime is
  local or remote

## `SessionDescriptor`

This is the lowest-level durable session noun Fireline should expose
independently of any higher-level `run` abstraction.

```ts
type SessionDescriptor = {
  sessionId: string
  runtimeKey: string
  runtimeId: string
  nodeId: string
  logicalConnectionId: string
  state: "active" | "broken" | "closed"
  supportsLoadSession: boolean
  traceId?: string
  parentPromptTurnId?: string
  createdAt: number
  updatedAt: number
  lastSeenAt: number
}
```

Ownership:

- produced durably onto the runtime's state stream
- materialized by `client.state`

Important boundary:

- session mutation is not owned by `client.sessions` yet
- at the low level, session creation and load happen through ACP
- durable session inspection happens through state

So:

- writes go through `client.acp`
- reads go through `client.state`

This split should remain explicit.

## `StreamEndpoint`

Durable streams are a first-class read surface.

```ts
type StreamEndpoint = Endpoint
```

Ownership:

- advertised either by `RuntimeDescriptor.state` or a future sibling stream
  contract

Read operations:

```ts
client.stream.open(endpoint)
client.stream.replay(endpoint, cursor?)
client.stream.live(endpoint, cursor?)
```

Why it matters:

- transcript views
- audit views
- durable operator dashboards
- replay and lineage reconstruction

This must not be reduced to "whatever the current WebSocket session emitted."

## `TopologySpec`

This is the public low-level noun for ACP proxy/tracer composition.

```ts
type TopologyComponentSpec = {
  name: string
  config?: Record<string, unknown>
}

type TopologySpec = {
  components: TopologyComponentSpec[]
}
```

Ownership:

- owned by `client.topology`
- consumed by `client.host.create(...)` or a future runtime initialize-time
  metadata path

Important rule:

- `TopologySpec` is the public API surface for proxy-chain composition
- ACP proxies are the implementation mechanism behind that surface

That means:

- no separate low-level `proxy` namespace
- no separate low-level `extensions` namespace
- no second config system that duplicates topology

Low-level operations:

```ts
client.topology.builder()
client.topology.parse(json)
client.topology.serialize(spec)
```

Likely later additions:

```ts
client.topology.listComponents(runtime?)
client.topology.describeComponent(name, runtime?)
client.topology.validate(runtime, spec)
```

These are still topology operations, not extension operations.

## `ResourceRef`

This is the **Resources** primitive in TypeScript form: a portable input that survives runtime changes by referencing a source rather than embedding its content.

```ts
type ResourceRef =
  | { kind: "local_path"; path: string; mount_path: string }
  | { kind: "git"; repo_url: string; ref?: string; mount_path: string }
  | { kind: "s3"; bucket: string; prefix: string; mount_path: string }
  | { kind: "gcs"; bucket: string; prefix: string; mount_path: string }
```

Ownership:

- passed as an array on `client.host.create({ resources: [...] })`
- interpreted by the runtime provider via a `ResourceMounter` trait on the Rust side
- no top-level namespace — Resources is a launch-spec field, not an action surface

Each ref pairs a `source_ref` (where to fetch from) with a `mount_path` (where to mount inside the runtime). This is the literal Anthropic interface for the Resources primitive: `[{source_ref, mount_path}]`. The list of mounter implementations grows additively — `LocalPathMounter` ships first (probably as a side effect of slice 13c Docker provider), `GitRemoteMounter` next, `S3Mounter` and `GcsMounter` later.

Important rule:

- the low-level surface accepts resource references and lets the runtime side resolve them
- consumers should never directly manage bind mounts, rsync, or snapshot transport details
- there is no `client.workspaces` namespace and there should not be one — workspace is a downstream product packaging concept, not a Fireline-owned object

The previous `WorkspaceRef` shape is replaced by `ResourceRef`. Existing consumers that use `WorkspaceRef` should migrate when slice 15 (Resources refactor) lands.

## `CapabilityRef`

This is the **Tools** primitive's portable reference shape — a stable launch input that points to tool sources and credential resolvers without injecting raw secrets.

```ts
type CapabilityRef =
  | { kind: "tool_ref"; tool: ToolSpec }
  | { kind: "credential_ref"; ref: string; scope?: string }
  | { kind: "policy_ref"; ref: string }
  | { kind: "topology_inline"; topology: TopologySpec }

type ToolSpec = {
  name: string
  description: string
  input_schema: JsonSchema
  transport_ref: TransportRef
  credential_ref?: CredentialRef
}

type TransportRef =
  | { kind: "mcp_url"; url: string }
  | { kind: "smithery"; catalog_url: string; tool_name: string }
  | { kind: "peer_runtime"; runtime_key: string }
  | { kind: "host_tool"; component_name: string }

type CredentialRef =
  | { kind: "env"; var: string }
  | { kind: "secret_store"; path: string }
  | { kind: "oauth_session"; provider: string; scope?: string }
```

Ownership:

- passed into runtime creation alongside topology
- resolved by conductor components on the runtime side at *call time*, not spawn time

Important rule:

- credentials never appear as values in the launch spec
- conductor-injected MCP/tool bridges resolve fresh auth headers when the tool is actually invoked
- runtimes never become credential vaults

This is the seam for systems like `agent.pw`, not a replacement for them. `agent.pw` (or any external credential broker) is what `credential_ref` resolves against.

## Orchestration by Composition and `resume(sessionId)`

Per [`managed-agents-mapping.md`](../explorations/managed-agents-mapping.md)
§2, Orchestration does not get a dedicated TS namespace. The primitive is
satisfied by composition of existing surfaces:

- read Session metadata from `client.state`
- check or provision runtime compute via `client.host`
- reconnect ACP via `client.acp`
- rebuild session state via `loadSession(sessionId)`

The helper shape is:

```ts
resume(sessionId: string): Promise<RuntimeDescriptor>
```

Conceptually:

```ts
async function resume(sessionId: string) {
  const session = await client.state.session.get(sessionId)

  let runtime = await client.host.get(session.runtimeKey)
  if (!runtime || runtime.status !== 'ready') {
    runtime = await client.host.create(session.runtimeSpec)
  }

  const acp = await client.acp.connect(runtime.acp)
  await acp.loadSession(sessionId)
  return runtime
}
```

This helper is not a new primitive. It is the documented composition surface
for the Anthropic `wake(session_id)` idea. The "scheduler" is any subscriber
loop that watches Session events and calls `resume(sessionId)` with normal
offset-based retry semantics.

Important rules:

- no dedicated `client.orchestration` namespace
- no dedicated `WakeReason` or `WakeReceipt` types
- no wake-specific control-plane endpoint in the TS contract
- `resume(sessionId)` depends on slice `14` persisting `runtimeSpec` as Session
  evidence at provision time
- retry semantics live in the subscriber loop and stream offset handling, not
  in a separate scheduler API

## `ApprovalRequest` (consumer of Orchestration composition)

`ApprovalRequest` is a consumer of the Orchestration composition, not a
primitive in its own right. It's a durable wait record that lives on the
Session stream and is resolved by appending a decision event plus calling
`resume(sessionId)` from a subscriber loop.

```ts
type ApprovalRequest = {
  requestId: string
  sessionId: string
  runtimeKey: string
  promptTurnId?: string
  kind: string
  title?: string
  state: "pending" | "resolved" | "expired" | "orphaned"
  options?: Array<{ optionId: string; name: string; kind: string }>
  createdAt: number
  resolvedAt?: number
}
```

Ownership:

- written durably by the `ApprovalGateComponent` conductor component on the
  runtime side
- read by any consumer subscribing to the Session stream (`client.state`
  materializes a `pending_approvals` view)
- serviced by an external resolver that appends an approval resolution event
  and then calls `resume(sessionId)` through a subscriber loop

Important rules:

- `ApprovalRequest` does not need its own namespace at the low level — it's
  served by `client.state` on the read side and `resume(sessionId)` on the
  resolution side
- a future ergonomic `client.approvals` wrapper at a higher layer is fine, but
  the primitive contract lives in Session + composition
- the `ApprovalGateComponent` is what makes a paused harness durable: it writes
  the pending record on suspend and rebuilds from the log on resume

This concretely demonstrates the layering: a primitive consumer (approvals) is
built from Session evidence, the `resume(sessionId)` composition, and one
conductor component (`ApprovalGateComponent` for the proxy-chain integration).

## Namespace Responsibilities

Each namespace below is annotated with the managed-agent primitive(s) it implements.

### `client.host` — Sandbox primitive (provision side)

Owns:

- runtime lifecycle (`provision` from the Sandbox primitive)
- runtime discovery
- runtime descriptors
- control-plane-facing creation and teardown

Does not own:

- ACP traffic (that's `client.acp` — the Sandbox `execute` side)
- transcript state (that's `client.state`)
- session prompt/update flow (that's `client.acp`)

Inputs it should accept on `create()`:

- agent launch input
- provider/placement input
- `TopologySpec`
- `ResourceRef[]` — the Resources primitive as a launch-spec field
- `CapabilityRef[]` — the Tools primitive's portable references

### `client.acp` — Sandbox primitive (execute side) + Harness I/O transport

Owns:

- ACP connection establishment against an advertised ACP endpoint
- initialize
- session create/load
- prompt and update flow (each prompt is one Sandbox `execute` against a long-lived runtime)
- direct protocol-level operations

Does not own:

- runtime discovery (that's `client.host`)
- durable session listing (that's `client.state`)
- topology metadata (that's `client.topology`)

Important rule:

- `client.acp` consumes an endpoint or attached transport — it must not perform hidden runtime lookup
- ACP traffic IS the Harness I/O channel — every effect the harness yields and every result that flows back travels through `client.acp` (or its server-side counterpart on the runtime)

### `client.state` — Session primitive (read side)

Owns:

- local materialization of durable Fireline state
- querying durable session/runtime/prompt-turn/permission/terminal evidence
- reactive subscriptions over durable state
- the canonical row schema that downstream products read

Does not own:

- ACP mutation flow (that's `client.acp`)
- runtime lifecycle (that's `client.host`)

Important rule:

- `client.state` is the durable read interface
- it should not be backed by a separate Fireline Rust query server — materialization happens client-side from the Session stream

### `client.stream` — Session primitive (raw stream access)

Owns:

- raw durable stream access
- replay/live consumption (`getEvents` from the Session primitive)
- low-level observation and sinks
- replay cursors and offset semantics

This is the escape hatch below `client.state`. Consumers who need lineage reconstruction, audit trails, or cross-runtime observation read directly from this layer.

### `client.topology` — Harness primitive (composition) + Tools primitive (registration)

Owns:

- `TopologySpec`
- conductor component composition — the proxy chain configuration
- tool registration via component attach
- parsing/serialization/validation
- component catalog introspection (later)

Does not own:

- runtime lifecycle (that's `client.host`)
- session lifecycle (that's `client.acp`)
- workload placement (that's `client.host`)

Important rule:

- topology is the public face of the conductor proxy chain — see [§Conductor and Proxy Chain](#conductor-and-proxy-chain)
- it is not a separate execution surface; it configures how the runtime composes around the harness's effects

### Orchestration — no dedicated namespace

Orchestration is intentionally not a namespace in the low-level TS surface.

It is satisfied by composition:

- `client.state` provides the durable Session reads
- `client.host` provides provision and runtime lookup
- `client.acp` provides reconnect plus `loadSession(sessionId)`
- `resume(sessionId)` ties those surfaces together

This keeps Orchestration aligned with the managed-agents reduction: the
"scheduler" is any subscriber loop that watches Session events and calls
`resume(sessionId)`.

## Reads vs Mutations

The low-level API should make read and mutation ownership obvious. Each operation below is tagged with its primitive.

### Mutations

- `client.host.create/stop/delete` — **Sandbox** (provision lifecycle)
- `client.acp.initialize` — **Sandbox** (execute channel setup)
- `client.acp.newSession/loadSession/prompt/...` — **Sandbox** (execute) + **Harness** (effect transport)
- later `client.topology.attach/detach` if dynamic topology becomes a thing — **Harness** + **Tools**

### Reads

- `client.host.get/list` — **Sandbox** (discovery)
- `client.state.open/...` — **Session** (read side)
- `client.stream.open/replay/live` — **Session** (raw stream access)
- later `client.topology.listComponents/describeComponent` — **Tools** (catalog introspection)

This split matters because reads increasingly come from durable evidence (the
Session primitive) while mutations increasingly go to control-plane or ACP
surfaces (Sandbox and Harness composition). Orchestration is composition across
those existing surfaces, not a separate mutation namespace.

The Anthropic primitive `emitEvent(id, event)` from the Session interface is **server-side only** in this contract. TypeScript clients consume events via reads; only the runtime side and conductor components emit them. This is the right asymmetry — clients should never bypass the conductor and write directly to a session log.

## What Is Not Yet A Low-Level Noun

These concepts are important, but they should stay above the low-level API
until the substrate is sharper:

- run
- workload
- profile
- workspace object
- extension preset
- cloud deployment package

Why:

- each of these composes multiple lower-level nouns
- introducing them too early will blur ownership boundaries
- Fireline still needs the substrate contract to settle first

## Practical Implications

The low-level API should make the happy path look like this:

```ts
// 1. PROVISION the Sandbox (Sandbox primitive, provision side)
const runtime = await client.host.create({
  agent: { command: 'codex' },
  placement: { provider: 'docker' },
  topology,                     // Harness + Tools composition
  resources: [                  // Resources primitive — launch-spec field
    { kind: 'git', repo_url: '...', mount_path: '/work' },
  ],
  capabilities: [               // Tools primitive — portable references
    { kind: 'tool_ref', tool: { name: 'review_pr', /* ... */ } },
  ],
})

// 2. EXECUTE against the Sandbox via ACP (Sandbox primitive, execute side)
//    Each prompt is one Sandbox execution, the runtime stays warm between
//    calls. ACP IS the Harness I/O channel.
const acp = await client.acp.connect(runtime.acp)
const session = await acp.newSession({ /* ... */ })
await session.prompt('Review the PR at ...')

// 3. READ the Session (Session primitive, read side)
const db = client.state.open({ endpoint: runtime.state })
const sessionRecord = await db.session.get(session.id)
const events = await db.session.events(session.id, { since: 0 })

// 4. RESUME later from a subscriber loop (Orchestration by composition)
//    The helper reads Session state, provisions if dormant, reconnects ACP,
//    and reloads the session.
await resume(session.id)
```

What matters is not the exact call syntax. What matters is the **primitive flow**:

1. **Sandbox.provision** — `client.host.create()` returns a `RuntimeDescriptor` carrying `acp` and `state` endpoint refs
2. **Sandbox.execute + Harness I/O** — `client.acp` opens an ACP connection against the descriptor's `acp` endpoint; every prompt is one effect the harness yields
3. **Session.getSession + getEvents** — `client.state` materializes durable rows from the descriptor's `state` endpoint
4. **Orchestration by composition** — `resume(sessionId)` composes Session
   reads, provision, ACP reconnect, and `loadSession`
5. **Tools + Harness composition** — `client.topology` is configured at provision time and shapes how the proxy chain handles every effect

Resources and Tools are not separate steps in this flow — they're inputs to step 1 (provision) that the runtime side resolves. They don't appear as first-class API calls because they're configuration, not actions.

## Recommendation

If Fireline wants the low-level TS API to stay coherent, it should freeze on these principles:

- **Anchor every namespace on a managed-agent primitive.** If a proposed namespace doesn't fit one of the six primitives, it belongs in a higher product layer.
- **Keep the public nouns small.** Five namespaces and a tight set of nouns
  (Endpoint, RuntimeDescriptor, SessionDescriptor, StreamEndpoint,
  TopologySpec, ResourceRef, CapabilityRef) plus a small number of helpers such
  as `resume(sessionId)`.
- **Make runtime descriptors the portability seam.** All endpoint discovery flows through `RuntimeDescriptor`.
- **Keep topology as the public proxy-chain primitive.** Conductor components are the implementation, topology is the configuration surface, and that surface implements both Harness composition and Tools registration.
- **Keep ACP and durable state as separate low-level surfaces.** Mutation flows through `client.acp`; reads flow through `client.state` and `client.stream`. Don't merge them into a single "session" namespace.
- **Do not add `client.orchestration`.** Orchestration is satisfied by
  composition; document and ship `resume(sessionId)` instead.
- **Introduce higher-level nouns only after these substrate contracts are stable.** `run`, `workload`, `profile`, `workspace object`, `extension preset` are all things downstream products should compose on top — not things Fireline should ship at the low level.

That gives higher layers like Flamecast room to build richer product APIs without forcing the Fireline systems layer to guess the wrong abstraction too early.

## Open questions

These are deliberately not pinned by this doc — they will be decided as slice
14, slice 15, and the first end-to-end consumer land.

1. **Where should `resume(sessionId)` live?** As a top-level helper export, a
   method under `client.state`, or a small utility module alongside ACP/state.
   The important rule is that it remains a helper over existing namespaces, not
   a new orchestration namespace.

2. **Should `client.topology` expose dynamic attach/detach, or is topology immutable per runtime?** Today topology is set at provision time only. Dynamic topology would require runtime-side reconfiguration of the proxy chain mid-session. Defer until a real consumer needs it.

3. **Where does `client.state` get its schema?** A Rust-side query server (rejected per the rule above), the TS-side `StreamDB` materializing rows itself (current approach), or a published schema package. Slice 14 will pin this.

4. **Is `Endpoint.headers` enough for auth, or do we need a separate auth primitive?** Today every endpoint carries its own bearer token in headers. A future model with rotating tokens, refresh, or mTLS will need richer shape. Open.

5. **How does `client.acp` handle reconnect?** The current shape assumes a
   long-lived WebSocket. With `resume(sessionId)` and dormant runtimes,
   reconnect becomes a more common path. Probably needs explicit
   `connectionState` and a reconnect strategy hook.
