# Client API Redesign — Functional Composition with Typed Multi-Agent Topologies

> **Status:** architectural proposal
> **Core idea:** `compose(sandbox, middleware, agent)` produces a `Harness`. Multiple harnesses compose via `peer()` / `fanout()` / `pipe()` into typed topologies the compiler can reason about.
> **Grounded in:**
> - [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md) — the Anthropic primitive table
> - [`../explorations/typescript-typed-functional-core-api.md`](../explorations/typescript-typed-functional-core-api.md) — "if the runtime executes it, model it as data"
> - [stream-db](https://durablestreams.com/stream-db) — reactive state layer
> - [Flamecast guides](https://flamecast.mintlify.app) — local/cloud/slackbot/webhook use cases
> - [Ramp Inspect](https://builders.ramp.com/post/why-we-built-our-background-agent) — background agent pattern

---

## 1. Package architecture

| Package | Owns | Plane |
|---|---|---|
| `@fireline/client` | Composition + provisioning | **Control plane** — `compose`, `peer`, `fanout`, `pipe`, `harness.start()` |
| `@fireline/state` | Reactive state observation | **Data plane** — stream-db over durable-streams, TanStack DB live queries |

ACP (`@agentclientprotocol/sdk`) is a third-party import. Fireline never wraps it. `SandboxHandle.acp` tells you where to connect; the ACP SDK tells you how to talk.

---

## 2. The three composable values

Everything starts with three independent, serializable values:

```typescript
// Fireline — composition
import { agent, sandbox, middleware, compose } from '@fireline/client'
// Fireline — middleware helpers (serializable specs, not closures)
import { trace, approve, budget } from '@fireline/client/middleware'
// Fireline — resource-ref helpers
import { localPath } from '@fireline/client/resources'

// An Agent — the ACP-speaking process to run
const myAgent = agent(['npx', '-y', '@anthropic-ai/claude-code-acp'])

// A Sandbox — the execution environment
const mySandbox = sandbox({
  resources: [localPath('~/project', '/workspace')],
  envVars: { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY! },
})

// A Middleware chain — interceptors applied to the ACP channel
const myMiddleware = middleware([
  trace(),
  approve({ scope: 'tool_calls', timeoutMs: 60_000 }),
  budget({ tokens: 500_000 }),
])
```

Each is a **serializable value** — data, not a closure. Each is independently reusable. Each is independently testable. None of them do anything until composed.

### `compose` — the fundamental operation

```typescript
const harness = compose(mySandbox, myMiddleware, myAgent)
// Type: Harness<'default'>
```

`compose(sandbox, middleware, agent)` produces a `Harness` — the runnable unit. A harness is everything the server needs to provision a sandbox, wire a conductor with the middleware chain, and start the agent process inside it.

```typescript
const handle = await harness.start({ serverUrl: 'http://localhost:4440' })
// handle.acp  → ACP WebSocket endpoint
// handle.state → durable state stream endpoint
```

`start()` sends the serialized `HarnessSpec` to the server as a single POST. The server provisions, wires, starts. One request, one handle.

**The type of each value:**

```typescript
type Agent = { readonly kind: 'agent'; readonly command: readonly string[] }

type SandboxSpec = {
  readonly kind: 'sandbox'
  readonly resources?: readonly ResourceRef[]
  readonly envVars?: Readonly<Record<string, string>>
  readonly image?: string
  readonly provider?: string
  readonly labels?: Readonly<Record<string, string>>
}

type MiddlewareChain = { readonly kind: 'middleware'; readonly chain: readonly Middleware[] }

type Harness<Name extends string = string> = {
  readonly kind: 'harness'
  readonly name: Name
  readonly sandbox: SandboxSpec
  readonly middleware: MiddlewareChain
  readonly agent: Agent
  start(opts: StartOptions): Promise<HarnessHandle<Name>>
}

interface HarnessHandle<Name extends string = string> {
  readonly name: Name
  readonly id: string
  readonly acp: Endpoint
  readonly state: Endpoint
}

interface StartOptions {
  readonly serverUrl: string
  readonly token?: string
  readonly name?: string
  readonly stateStream?: string
  readonly startupTimeoutMs?: number
}
```

### Export structure

```
@fireline/client              compose, agent, sandbox, middleware, peer, fanout, pipe, Sandbox
@fireline/client/middleware   trace, approve, budget, inject, peer (middleware-level)
@fireline/client/resources    localPath, streamBlob, gitRepo, ociImage, httpUrl
@fireline/state               createFirelineDB, FirelineDB, useLiveQuery (re-exported from @tanstack/react-db)
```

### Resource-ref convenience helpers (`@fireline/client/resources`)

```typescript
// Each returns a ResourceRef — { source_ref, mount_path, read_only? }
function localPath(path: string, mountPath: string, readOnly?: boolean): ResourceRef
function streamBlob(stream: string, key: string, mountPath: string): ResourceRef
function gitRepo(url: string, ref: string, mountPath: string): ResourceRef
function ociImage(image: string, path: string, mountPath: string): ResourceRef
function httpUrl(url: string, mountPath: string): ResourceRef
```

These are thin constructors over `ResourceRef`. They exist so examples read cleanly; callers can also construct the literal objects.

---

## 3. Type-level composition for multi-agent topologies

### Single agent — `compose`

```typescript
// Fireline
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, approve } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

const codebase = localPath('~/projects/frontend', '/workspace', true)

const reviewer = compose(
  sandbox({ resources: [codebase] }),
  middleware([trace(), approve({ scope: 'tool_calls' })]),
  agent(['claude-code-acp']),
).as('reviewer')
// Type: Harness<'reviewer'>
```

The `.as(name)` method gives the harness a compile-time name. This name is used in topology operators to type-check references.

### Multi-agent with peering — `peer`

```typescript
// Fireline
import { compose, agent, sandbox, middleware, peer } from '@fireline/client'
import { trace, approve } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

const codebase = localPath('~/projects/frontend', '/workspace', true)

const reviewer = compose(
  sandbox({ resources: [codebase] }),
  middleware([trace(), approve({ scope: 'tool_calls' })]),
  agent(['claude-code-acp']),
).as('reviewer')

const notifier = compose(
  sandbox({}),
  middleware([trace()]),
  agent(['slack-notifier']),
).as('notifier')

// peer() connects agents — the type system knows the shape
const topology = peer(reviewer, notifier)
// Type: Topology<{ reviewer: Harness<'reviewer'>; notifier: Harness<'notifier'> }>

const handles = await topology.start({ serverUrl: 'http://localhost:4440' })
// handles.reviewer.acp  — ACP endpoint for the reviewer
// handles.notifier.acp  — ACP endpoint for the notifier
// reviewer can call notifier via ACP peer calls
// cross-agent causality is visible through ACP _meta trace context and the trace backend
```

**Type safety:** `handles.reviewer` is typed. `handles.nonexistent` is a compile error. You can't `peer` a harness that doesn't exist; the TypeScript compiler catches the topology error.

### Fan-out — `fanout`

```typescript
const workers = fanout(
  compose(sandbox({...}), middleware([trace()]), agent(['worker'])).as('worker'),
  { count: 3 },
)
// Type: Fanout<Harness<'worker'>, 3>

const handles = await workers.start({ serverUrl })
// handles[0].acp, handles[1].acp, handles[2].acp
```

Three instances of the same harness. Each gets its own sandbox, its own state stream, its own ACP endpoint. The server provisions them in parallel.

### Pipeline — `pipe`

```typescript
const pipeline = pipe(
  compose(sandbox({...}), middleware([trace()]), agent(['researcher'])).as('researcher'),
  compose(sandbox({...}), middleware([trace()]), agent(['writer'])).as('writer'),
)
// Type: Pipeline<[Harness<'researcher'>, Harness<'writer'>]>

const handles = await pipeline.start({ serverUrl })
// researcher's output feeds writer's input through the pipeline wiring
```

Sequential composition: the first harness's output becomes the next harness's input. The `pipe` operator establishes that handoff without introducing a tenant-wide lineage stream.

### Type signatures

```typescript
function compose(sandbox: SandboxSpec, middleware: MiddlewareChain, agent: Agent): Harness
function peer<T extends Record<string, Harness>>(...harnesses: Harness[]): Topology<T>
function fanout<H extends Harness, N extends number>(harness: H, opts: { count: N }): Fanout<H, N>
function pipe<H extends Harness[]>(...harnesses: H): Pipeline<H>
```

The return types carry the topology shape at the type level. TypeScript's structural typing means you get autocomplete on `handles.reviewer`, compile-time errors on `handles.typo`, and IDE support for navigating multi-agent topologies.

---

## 4. How this maps to the wire

Every composition function produces a **serializable spec**:

```typescript
compose() → HarnessSpec { sandbox: SandboxSpec, middleware: Middleware[], agent: Agent }
peer()    → TopologySpec { kind: 'peer', harnesses: Record<string, HarnessSpec> }
fanout()  → TopologySpec { kind: 'fanout', harness: HarnessSpec, count: number }
pipe()    → TopologySpec { kind: 'pipe', stages: HarnessSpec[] }
```

`harness.start()` sends the `HarnessSpec` as a single `POST /v1/sandboxes`. `topology.start()` sends the `TopologySpec` as `POST /v1/topologies` — the server provisions all sandboxes and wires peer edges between them.

For single-harness `start()`, the server response is a `SandboxDescriptor` mapped to a `HarnessHandle`. For multi-harness `topology.start()`, the server response is a `TopologyDescriptor` with a handle per harness name.

**The execute primitive still exists:**

```typescript
const sandbox = new Sandbox({ serverUrl })
const result = await sandbox.execute(handle, 'ls -la /workspace')
```

`execute` is a standalone verb on the `Sandbox` class, not on the `Harness`. It's the Anthropic primitive's second verb — "call many times as a tool." The harness is how you set up the sandbox; `execute` is how you use it. They compose but don't merge.

---

## 5. The seven combinators as middleware + topology operators

The managed-agents-mapping doc defines seven combinators. Under the composition model, they split into two categories:

### Middleware (per-agent, intercepts the ACP channel)

| Combinator | Middleware helper | What it does |
|---|---|---|
| `observe` | `trace()` | Log every ACP effect to the durable stream |
| `mapEffect` | `inject(sources)` | Prepend context to prompts |
| `appendToSession` | *(always on — the `DurableStreamTracer` is implicit)* | Append effects to the session log |
| `filter` | `budget({ tokens })` | Hard budget cap — reject effects over budget |
| `suspend` | `approve({ scope, timeoutMs })` | Require approval before forwarding tool calls |

### Topology operators (multi-agent, defines the graph)

| Combinator | Operator | What it does |
|---|---|---|
| `substitute` | `peer(reviewer, notifier)` | Route peer calls between agents. `peer()` is both a topology operator (wiring the graph) and a middleware effect (the `PeerComponent` intercepts `session/new` and injects per-session MCP tools). |
| `fanout` | `fanout(harness, { count })` | Spawn N instances of the same harness for parallel work |

The **type system distinguishes them**: `Middleware` is a value inside `middleware([...])`. `peer()` and `fanout()` are functions over `Harness` values. You can't accidentally put a topology operator inside a middleware chain — it's a type error.

---

## 6. Use case examples with `compose`

### 6.1 Local agent dev

```typescript
// Fireline
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'
// ACP (third-party)
import { ClientSideConnection, PROTOCOL_VERSION } from '@agentclientprotocol/sdk'

const handle = await compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([trace()]),
  agent(['node', 'agent.js']),
).start({ serverUrl: 'http://localhost:4440', name: 'dev-agent' })

// ACP session — third-party SDK, not Fireline
const ws = new WebSocket(handle.acp.url)
const conn = new ClientSideConnection(/* handler */, createWebSocketStream(ws))
await conn.initialize({ protocolVersion: PROTOCOL_VERSION, clientInfo: { name: 'dev', version: '0.0.1' }, clientCapabilities: {} })
const { sessionId } = await conn.newSession({ cwd: '/workspace' })
await conn.prompt({ sessionId, prompt: [{ type: 'text', text: 'Review the README' }] })
```

### 6.2 Cloud deployment — same code, different URL

```typescript
// Fireline
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, approve } from '@fireline/client/middleware'
import { streamBlob } from '@fireline/client/resources'

// Same composition, different serverUrl. That's the whole migration.
const handle = await compose(
  sandbox({ provider: 'docker', resources: [streamBlob('resources:prod', 'codebase', '/workspace')] }),
  middleware([trace(), approve({ scope: 'tool_calls' })]),
  agent(['claude-code-acp']),
).start({ serverUrl: 'https://fireline.prod.internal' })
```

### 6.3 Slackbot — fire-and-forget prompt, stream-based observation

```typescript
// Fireline
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace } from '@fireline/client/middleware'
import { createFirelineDB } from '@fireline/state'
// ACP (third-party)
import { ClientSideConnection, PROTOCOL_VERSION, type RequestId } from '@agentclientprotocol/sdk'
// User's integration (NOT Fireline)
import { App } from '@slack/bolt'

const app = new App({ token: process.env.SLACK_BOT_TOKEN!, signingSecret: process.env.SLACK_SIGNING_SECRET! })

app.event('app_mention', async ({ event, say }) => {
  // 1. Provision a sandbox
  const handle = await compose(
    sandbox({}),
    middleware([trace()]),
    agent(['claude-code-acp']),
  ).start({ serverUrl: process.env.FIRELINE_URL!, name: `slack-${event.ts}` })

  // 2. Fire the prompt — don't await, observation is via the stream
  const ws = new WebSocket(handle.acp.url)
  const conn = new ClientSideConnection(/* handler */, createWebSocketStream(ws))
  await conn.initialize({ protocolVersion: PROTOCOL_VERSION, clientInfo: { name: 'slackbot', version: '0.0.1' }, clientCapabilities: {} })
  const { sessionId } = await conn.newSession({ cwd: '/' })
  const requestId = `slack:${event.ts}` as RequestId
  void conn.prompt({ sessionId, requestId, prompt: [{ type: 'text', text: event.text }] })  // fire-and-forget

  // 3. Observe completion + permissions via @fireline/state (the ONLY subscription interface)
  const db = createFirelineDB({ stateStreamUrl: handle.state.url })
  await db.preload()
  db.collections.promptRequests.subscribe((requests) => {
    const completed = requests.find(r => r.sessionId === sessionId && r.requestId === requestId && r.completedAt)
    if (completed) {
      say(`Agent finished: ${completed.stopReason}`)  // say() is Slack Bolt's reply helper
    }
  })
  db.collections.permissions.subscribe((perms) => {
    const pending = perms.find(p => p.sessionId === sessionId && p.state === 'pending')
    if (pending) {
      say(`Agent needs approval: ${pending.title ?? 'permission request'}`)
    }
  })
})
```

### 6.4 Background agent — fire and forget

```typescript
// Fireline
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, approve } from '@fireline/client/middleware'
import { gitRepo } from '@fireline/client/resources'

// User's variables
const repoUrl = 'https://github.com/org/repo'
const branch = 'main'
const taskId = 'task-42'

await compose(
  sandbox({ resources: [gitRepo(repoUrl, branch, '/workspace')], labels: { task: taskId } }),
  middleware([trace(), approve({ scope: 'all' })]),
  agent(['claude-code-acp']),
).start({ serverUrl: 'https://fireline.prod.internal', name: `task-${taskId}` })
// User monitors via @fireline/state in a dashboard — no blocking here
```

### 6.5 Multi-agent review pipeline

```typescript
// Fireline
import { compose, agent, sandbox, middleware, pipe } from '@fireline/client'
import { trace, approve } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

const codebase = localPath('~/projects/frontend', '/workspace', true)  // read-only

const reviewer = compose(
  sandbox({ resources: [codebase] }),
  middleware([trace(), approve({ scope: 'tool_calls' })]),
  agent(['claude-code-acp']),
).as('reviewer')

const writer = compose(
  sandbox({ resources: [codebase] }),
  middleware([trace()]),
  agent(['claude-code-acp']),
).as('writer')

const handles = await pipe(reviewer, writer).start({ serverUrl: 'http://localhost:4440' })
// reviewer runs first; writer observes the prior agent-plane events through Fireline's state substrate
```

### 6.6 Reactive observation — zero polling

```typescript
// Fireline — state observation
import { createFirelineDB } from '@fireline/state'
// TanStack DB (peer dependency of @fireline/state)
import { useLiveQuery } from '@tanstack/react-db'

// handle obtained from a prior compose(...).start() call
const db = createFirelineDB({ stateStreamUrl: handle.state.url })
const promptRequests = useLiveQuery(q =>
  q.from({ r: db.collections.promptRequests }).where(({ r }) => r.sessionId === sessionId && r.requestId === requestId)
)
// The stream IS the API. No fetch. No setInterval. React re-renders as events arrive.
```

### 6.7 Multi-agent topology — per-agent streams, trace-owned lineage

```typescript
// Fireline
import { compose, agent, sandbox, middleware, peer } from '@fireline/client'
import { trace } from '@fireline/client/middleware'
import { createFirelineDB } from '@fireline/state'

// reviewer and notifier defined as in §6.5
const handles = await peer(reviewer, notifier).start({ serverUrl: 'http://localhost:4440' })

const reviewerDb = createFirelineDB({ stateStreamUrl: handles.reviewer.state.url })
const notifierDb = createFirelineDB({ stateStreamUrl: handles.notifier.state.url })
// reviewerDb.collections.sessions       → reviewer's agent-plane state
// notifierDb.collections.promptRequests → notifier's prompt requests
// peer-call causality is carried in ACP _meta.traceparent / tracestate / baggage
// "reviewer called notifier" is queried from the trace backend, not Fireline rows
```

---

## 7. What `@fireline/state` does in a topology

**Still agent-plane only. One stream URL per agent.**

Each agent in a topology writes to its own agent-plane stream. `@fireline/state` stays focused on agent-plane collections such as sessions, prompt requests, permissions, and chunks; it does not materialize cross-agent lineage rows, and it does not expose infra-plane rows on the client surface.

```typescript
// Fireline — state observation
import { createFirelineDB } from '@fireline/state'

const reviewerDb = createFirelineDB({ stateStreamUrl: handles.reviewer.state.url })
const notifierDb = createFirelineDB({ stateStreamUrl: handles.notifier.state.url })
// Dashboards join agent-plane views across streams as needed.
// Cross-agent causality comes from ACP _meta trace context in the trace backend.
```

Multi-agent topologies expose deployment-wide agent-plane visibility through Fireline state. Dashboards see sessions, prompt requests, permissions, and chunks across the deployment by subscribing to the relevant agent-plane streams. Cross-agent causality is not materialized as Fireline rows; it is queried through W3C Trace Context and the configured trace backend.

The topology changes the **call graph** (which agents can peer-call each other), not the **state model**. Fireline state stays flat and agent-local; outbound peer calls inject `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage`, inbound peer calls extract them, and the observability layer owns the span graph.

**No Orchestrator class.** Orchestration IS stream subscription. If a session needs advancing (e.g., a pending approval resolved), the subscriber reacts:

```typescript
// Fireline — state observation
import { createFirelineDB } from '@fireline/state'
// Fireline — stream write helpers (for external event appends like approval responses)
import { appendApprovalResolved } from '@fireline/client/events'

db.collections.permissions.subscribe((perms) => {
  for (const perm of perms.filter(p => p.state === 'pending')) {
    // appendApprovalResolved is a thin wrapper over a durable-streams producer append
    appendApprovalResolved({ streamUrl: db.stateStreamUrl, sessionId: perm.sessionId, requestId: perm.requestId, allow: true })
  }
})
```

If someone needs a wake loop, they subscribe and react. The primitive is the stream, not a `whileLoopOrchestrator` wrapper.

---

## 8. Module layout — old → new

```
packages/client/src/                    packages/client/src/
├── host.ts              DELETED        ├── compose.ts          NEW (compose, agent, sandbox, middleware)
├── host/                DELETED        ├── topology.ts         UPDATED (peer, fanout, pipe operators)
├── host-fireline/       DELETED        ├── sandbox.ts          NEW (Sandbox class — provision + execute)
├── host-hosted-api/     DELETED        ├── types.ts            NEW (specs, handles, middleware union)
├── sandbox/             MERGED         │
├── orchestration/       DELETED        │
│                                       │
├── core/                UPDATED        ├── core/               UPDATED (middleware helpers: trace, approve, budget, inject, peer)
├── sandbox-local/       KEPT           ├── sandbox-local/      KEPT
├── catalog.ts           KEPT           ├── catalog.ts          KEPT
├── acp.ts               KEPT           ├── acp.ts              KEPT
└── index.ts             UPDATED        └── index.ts            UPDATED
```

`@fireline/state` is unchanged. It already does the right thing.

---

## 9. Migration plan

**M1 (1 day):** Ship `compose`, `agent`, `sandbox`, `middleware` functions + `Harness` type in `@fireline/client/v2`. Single-harness `start()` targets existing `/v1/runtimes` with field mapping. Zero server changes.

**M2 (half day):** Rewire browser harness from `createFirelineHost` to `compose(...).start(...)`.

**M3 (1 day):** Add `peer()`, `fanout()`, `pipe()` operators. Server-side: add `POST /v1/topologies` endpoint that provisions multiple sandboxes and wires peer edges. This is the only server change.

**M4 (half day):** Delete old surface (`host/`, `host-fireline/`, `host-hosted-api/`, `sandbox/`, `orchestration/`).

**M5 (separate):** Server-side endpoint rename `/v1/runtimes` → `/v1/sandboxes`.

**Total: ~3 days for the core (M1-M2), +1.5 days for topology operators (M3-M4).**

---

## 10. Composition with other proposals

| Proposal | Composition |
|---|---|
| **Cross-host discovery** | `peer()` across server URLs: `peer(compose(...).at('server-a'), compose(...).at('server-b'))`. The `at()` method pins a harness to a specific server. |
| **Resource discovery** | `sandbox({ resources: [streamBlob('resources:tenant-demo', 'codebase')] })` — resource refs in the sandbox spec, resolved server-side. |
| **Secrets injection** | `middleware([secretsProxy({ OPENAI_KEY: { allow: 'api.openai.com' } })])` — a middleware entry, not a sandbox concern. |
| **Stream-FS** | `sandbox({ resources: [streamFs('snapshot-id', { mode: 'liveReadWrite' })] })` — another resource ref variant. |
| **Sandbox provider model** | `sandbox({ provider: 'microsandbox' })` — provider hint in the sandbox spec, resolved by the server's `ProviderDispatcher`. |
| **TLA verification** | The composition operators produce serializable specs. A TLA model can check topology-level invariants: "every peer edge is bidirectional", "fanout count > 0", "pipeline handoff preserves stage order and agent-plane state visibility." |

---

## Appendix: operator surface — `SandboxAdmin`

The primitive API is `compose` + `start` + `execute`. Operators need more. Admin lives on a separate class:

```typescript
const admin = new SandboxAdmin({ serverUrl })
await admin.destroy(handle.id)
const all = await admin.list({ team: 'frontend' })
const status = await admin.status(handle.id)
```

Not on the primitive surface. Not in the import path for agent builders. Exists for dashboards, CLIs, and the browser harness.
