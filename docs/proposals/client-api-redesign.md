# Client API Redesign — Full Primitive Surface

> **Status:** architectural proposal (full-scope — covers `@fireline/client`, `@fireline/state`, ACP, and stream-db composition)
> **Replaces:** the current multi-layer TS client surface
> **Companion:** [`./sandbox-provider-model.md`](./sandbox-provider-model.md) — the Rust-side provider model
> **Grounded in:**
> - [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md) — the Anthropic primitive table
> - [Flamecast guides](https://flamecast.mintlify.app) — local agents, cloud agents, slackbot, webhooks
> - [stream-db](https://durablestreams.com/stream-db) — the reactive layer over durable-streams
> - [Ramp Inspect](https://builders.ramp.com/post/why-we-built-our-background-agent) — the background agent pattern

---

## 1. Package architecture

Two packages. They stay separate. Each owns one plane.

| Package | Owns | Plane |
|---|---|---|
| `@fireline/client` | Sandbox lifecycle + declarative config | **Control plane** — provision, execute, compose topology |
| `@fireline/state` | Reactive state observation | **Data plane** — stream-db over durable-streams, TanStack DB live queries |

They compose through `SandboxHandle`:

```typescript
import { Sandbox } from '@fireline/client'
import { createFirelineDB } from '@fireline/state'

const sandbox = new Sandbox({ serverUrl: 'http://localhost:4440' })
const handle = await sandbox.provision({ name: 'my-agent', agentCommand: [...] })

// Control plane gave you the handle.
// Data plane uses handle.state to subscribe.
const db = createFirelineDB({ stateStreamUrl: handle.state.url })
```

**Why not merge?** Because `@fireline/state` is useful without `@fireline/client`. A dashboard that only reads state from a durable stream doesn't need to provision sandboxes. A CLI that only provisions doesn't need TanStack DB. The two planes are independently useful.

ACP (`@agentclientprotocol/sdk`) is a third-party package the user imports directly. Fireline never wraps it. The `SandboxHandle.acp` endpoint tells you where to connect; the ACP SDK tells you how to talk. No side channels.

---

## 2. The primitive client surfaces

The Anthropic managed-agent table defines six primitives. Each one maps to a specific surface — a method, a config field, or a separate plane:

| # | Primitive | Client surface | Where it lives |
|---|---|---|---|
| 1 | **Session** | Not a client verb. Sessions are ACP-plane via `handle.acp`. | `@agentclientprotocol/sdk` — `ClientSideConnection.newSession()` |
| 2 | **Orchestration** | `wake(session_id) → void` — separate primitive. | `@fireline/client` — standalone `Orchestrator` interface (unchanged from current) |
| 3 | **Harness** | Not a client surface. Runs inside the sandbox, server-side. | The conductor proxy chain inside the `fireline` binary |
| 4 | **Sandbox** | `provision(config) → SandboxHandle`, `execute(handle, input) → string` | `@fireline/client` — the `Sandbox` class |
| 5 | **Resources** | Declarative config field: `SandboxConfig.resources: ResourceRef[]` | `@fireline/client` — `SandboxConfig` type |
| 6 | **Tools** | Declarative config field: `SandboxConfig.topology` carries tool registrations via combinator helpers | `@fireline/client` — combinator system in `core/` |

**Three primitives are methods** (Sandbox: `provision` + `execute`; Orchestration: `wake`).
**Two primitives are config fields** (Resources, Tools — declared in `SandboxConfig`, interpreted server-side).
**Two primitives are separate planes** (Session via ACP, Harness server-side).
**Zero primitives require polling or status checks** — the durable stream IS the observation layer.

### The Sandbox primitive

```typescript
class Sandbox {
  constructor(opts: { serverUrl: string; token?: string; startupTimeoutMs?: number })

  /** Provision — hand me a place where an agent can run. */
  provision(config: SandboxConfig): Promise<SandboxHandle>

  /** Execute — run a command inside it. Returns stdout. */
  execute(handle: SandboxHandle, input: string): Promise<string>
}

interface SandboxHandle {
  readonly id: string
  readonly acp: Endpoint      // connect here for ACP sessions
  readonly state: Endpoint    // subscribe here for durable state
}
```

Two methods. The handle carries the two endpoints the caller needs to reach the other planes.

### The Orchestration primitive

```typescript
interface Orchestrator {
  wakeOne(session_id: string): Promise<void>
  start(): Promise<void>
  stop(): Promise<void>
}

function whileLoopOrchestrator(opts: {
  handler: (session_id: string) => Promise<void>
  registry: SessionRegistry
  pollIntervalMs?: number
}): Orchestrator
```

Unchanged from the current `@fireline/client/orchestration`. `wake` is a separate primitive from `Sandbox` — they compose but don't merge.

---

## 3. The state observation layer

**The stream IS the observation layer.** No `sandbox.status()` polling. No `host.wake()` checking. The developer subscribes to the durable stream via `@fireline/state` and gets a reactive view of everything happening inside the sandbox.

```typescript
import { createFirelineDB } from '@fireline/state'
import { useLiveQuery } from '@tanstack/react-db'
import { eq } from '@tanstack/db'

const db = createFirelineDB({ stateStreamUrl: handle.state.url })
await db.preload()  // replays from beginning, then stays connected for live updates

// Reactive queries — update automatically as the stream advances
function AgentDashboard({ sessionId }: { sessionId: string }) {
  const turns = useLiveQuery(q =>
    q.from({ turns: db.collections.promptTurns })
      .where(({ turns }) => eq(turns.sessionId, sessionId))
  )

  const permissions = useLiveQuery(q =>
    q.from({ perms: db.collections.permissions })
      .where(({ perms }) => eq(perms.sessionId, sessionId))
      .where(({ perms }) => eq(perms.state, 'pending'))
  )

  const chunks = useLiveQuery(q =>
    q.from({ chunks: db.collections.chunks })
      .where(({ chunks }) => eq(chunks.promptTurnId, currentTurnId))
  )

  return (
    <>
      <TurnList turns={turns} />
      <ChunkStream chunks={chunks} />
      <PendingPermissions permissions={permissions} />
    </>
  )
}
```

**This is stream-db in action.** `createFirelineDB` is a thin wrapper around `createStreamDB` from `@durable-streams/state` with Fireline's schema pre-wired. TanStack DB provides differential dataflow — queries update incrementally as new events arrive on the stream. No polling. No `setInterval`. No manual refetch.

**What `@fireline/state` provides (today — unchanged):**

| Collection | Entity type | Primary key | What it shows |
|---|---|---|---|
| `sessions` | `session` | `sessionId` | Active sessions with runtime key, state, timestamps |
| `promptTurns` | `prompt_turn` | `promptTurnId` | Each prompt/response turn with state, stop reason |
| `chunks` | `chunk` | `chunkId` | Streaming content blocks per turn |
| `connections` | `connection` | `logicalConnectionId` | ACP connection lifecycle |
| `permissions` | `permission` | `requestId` | Permission requests and resolutions |
| `childSessionEdges` | `child_session_edge` | `edgeId` | Cross-agent call lineage |
| `runtimeInstances` | `runtime_instance` | `instanceId` | Sandbox process lifecycle |
| `pendingRequests` | `pending_request` | `requestId` | In-flight ACP requests |

Plus derived collections: `createSessionTurnsCollection`, `createActiveTurnsCollection`, `createPendingPermissionsCollection`, `createTurnChunksCollection`, etc.

---

## 4. Declarative composition

Resources, tools, topology, and secrets all compose **declaratively** in `SandboxConfig`. Nothing is imperative. Nothing requires a second API call after `provision`.

```typescript
import { topology, durableTrace, approvalGate, contextInjection, peer, budget } from '@fireline/client/core'

const config: SandboxConfig = {
  name: 'reviewer-agent',
  agentCommand: ['npx', '-y', '@anthropic-ai/claude-code-acp'],

  // TOPOLOGY — combinator chain interpreted by the conductor inside the sandbox
  topology: topology(
    durableTrace(),                                         // observe: log every effect
    contextInjection([{ kind: 'workspace_file', path: '/workspace/README.md' }]),  // mapEffect: prepend context
    approvalGate({ scope: 'tool_calls', timeoutMs: 60_000 }),  // suspend: require approval for tool calls
    budget({ tokens: 500_000 }),                            // filter: hard budget cap
    peer(['agent:slack-notifier']),                          // substitute: route peer calls
  ),

  // RESOURCES — what to mount inside the sandbox
  resources: [
    { source_ref: { kind: 'localPath', host_id: 'self', path: '~/projects/frontend' }, mount_path: '/workspace', read_only: true },
    { source_ref: { kind: 'durableStreamBlob', stream: 'resources:tenant-demo', key: 'shared-config' }, mount_path: '/config' },
    { source_ref: { kind: 'gitRepo', url: 'https://github.com/org/repo', ref: 'main', path: '/' }, mount_path: '/reference' },
  ],

  // ENVIRONMENT — injected into the agent process
  envVars: {
    ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY!,
    WORKSPACE_ROOT: '/workspace',
  },

  // LABELS — for operator lookup and pool reuse
  labels: { team: 'frontend', env: 'dev' },
}

const handle = await sandbox.provision(config)
```

**Everything the sandbox needs to run is in one declarative object.** The server interprets the topology, mounts the resources, injects the env vars, and starts the agent. The client sends one POST and gets back a handle. This matches Ramp Inspect's insight: *"agents should never be limited by missing context or tools, but only by model intelligence itself"* — so we front-load all context into the config.

---

## 5. Use case mapping

Seven external references, seven code examples. Each under 15 lines. Each touches at most 2 packages.

### 5.1 Local agent dev loop ([Flamecast: local-agents](https://flamecast.mintlify.app/guides/local-agents))

```typescript
import { Sandbox } from '@fireline/client'

const sandbox = new Sandbox({ serverUrl: 'http://localhost:4440' })
const handle = await sandbox.provision({
  name: 'dev-agent',
  agentCommand: ['node', 'agent.js'],
  resources: [{ source_ref: { kind: 'localPath', host_id: 'self', path: '.' }, mount_path: '/workspace' }],
})

// ACP session — standard SDK, no wrapper
const conn = await acpConnect(handle.acp.url)
const { sessionId } = await conn.newSession({ cwd: '/workspace' })
await conn.prompt({ sessionId, prompt: [{ type: 'text', text: 'Review the README' }] })
```

### 5.2 Cloud deployment ([Flamecast: cloud-agents](https://flamecast.mintlify.app/guides/cloud-agents))

```typescript
// Same code, different server URL. That's the whole migration.
const sandbox = new Sandbox({ serverUrl: 'https://fireline.prod.internal:4440' })
const handle = await sandbox.provision({
  name: 'cloud-agent',
  agentCommand: ['npx', '-y', '@anthropic-ai/claude-code-acp'],
  provider: 'docker',  // or 'microsandbox' — server picks if omitted
  resources: [{ source_ref: { kind: 'durableStreamBlob', stream: 'resources:tenant-prod', key: 'codebase' }, mount_path: '/workspace' }],
})
```

### 5.3 Custom agent build ([Flamecast: build-your-own-agent](https://flamecast.mintlify.app/guides/build-your-own-agent))

```typescript
// Your agent implements ACP. Fireline provisions + manages it.
const handle = await sandbox.provision({
  name: 'my-custom-agent',
  agentCommand: ['./my-agent', '--port', '9100'],
  topology: topology(durableTrace(), approvalGate({ scope: 'all' })),
})
// That's it. Fireline handles ACP handshake, session management, durable tracing.
```

### 5.4 Slackbot integration ([Flamecast: slackbot](https://flamecast.mintlify.app/guides/slackbot))

```typescript
import { Sandbox } from '@fireline/client'

const sandbox = new Sandbox({ serverUrl: process.env.FIRELINE_URL! })

app.event('app_mention', async ({ event }) => {
  const handle = await sandbox.provision({ name: `slack-${event.ts}`, agentCommand: ['claude-code-acp'] })
  const conn = await acpConnect(handle.acp.url)
  const { sessionId } = await conn.newSession({ cwd: '/' })
  const response = await conn.prompt({ sessionId, prompt: [{ type: 'text', text: event.text }] })
  await say(formatResponse(response))
})
```

### 5.5 Webhook-driven orchestration ([Flamecast RFC: webhooks](https://flamecast.mintlify.app/rfcs/webhooks))

```typescript
import { Sandbox } from '@fireline/client'
import { createFirelineDB } from '@fireline/state'

const sandbox = new Sandbox({ serverUrl: process.env.FIRELINE_URL! })
const handle = await sandbox.provision({ name: 'webhook-agent', agentCommand: [...] })

// Subscribe to permission requests via the durable stream — NO POLLING
const db = createFirelineDB({ stateStreamUrl: handle.state.url })
await db.preload()
db.collections.permissions.subscribe((perms) => {
  for (const perm of perms.filter(p => p.state === 'pending')) {
    // POST webhook to the client's callback URL
    fetch(callbackUrl, { method: 'POST', body: JSON.stringify(perm) })
  }
})
```

### 5.6 Background agent ([Ramp: why-we-built-our-background-agent](https://builders.ramp.com/post/why-we-built-our-background-agent))

```typescript
import { Sandbox } from '@fireline/client'

const sandbox = new Sandbox({ serverUrl: 'https://fireline.prod.internal' })

// Fire-and-forget: provision → prompt → walk away
const handle = await sandbox.provision({
  name: `inspect-${taskId}`,
  agentCommand: ['claude-code-acp'],
  resources: [{ source_ref: { kind: 'gitRepo', url: repoUrl, ref: branch, path: '/' }, mount_path: '/workspace' }],
  labels: { task: taskId, user: userId },
})
const conn = await acpConnect(handle.acp.url)
const { sessionId } = await conn.newSession({ cwd: '/workspace' })
await conn.prompt({ sessionId, prompt: [{ type: 'text', text: taskDescription }] })
// User monitors progress via @fireline/state in a dashboard — no blocking
```

### 5.7 Reactive agent pattern (reactive UI observing agent work)

```typescript
import { createFirelineDB } from '@fireline/state'
import { useLiveQuery } from '@tanstack/react-db'

// The agent is already running. The UI just subscribes.
function AgentMonitor({ stateUrl, sessionId }: Props) {
  const db = useMemo(() => createFirelineDB({ stateStreamUrl: stateUrl }), [stateUrl])
  const turns = useLiveQuery(q =>
    q.from({ t: db.collections.promptTurns }).where(({ t }) => eq(t.sessionId, sessionId))
  )
  const chunks = useLiveQuery(q =>
    q.from({ c: db.collections.chunks }).where(({ c }) => eq(c.promptTurnId, turns[turns.length - 1]?.promptTurnId))
  )
  return <StreamingView turns={turns} chunks={chunks} />
}
// ZERO fetch calls. ZERO polling. The stream IS the API.
```

**Every example: ≤15 lines. At most 2 packages (`@fireline/client` + `@fireline/state` or `@agentclientprotocol/sdk`).** The common pattern: `new Sandbox(url)` → `sandbox.provision(config)` → either ACP for sessions or `@fireline/state` for observation. No third path.

---

## 6. What gets deleted / what stays / what's new

```
packages/client/src/                    packages/client/src/
├── host.ts              DELETED        ├── sandbox.ts          NEW (Sandbox class — 2 methods)
├── host/                DELETED        ├── admin.ts            NEW (SandboxAdmin — operator extensions)
├── host-fireline/       DELETED        ├── types.ts            NEW (SandboxConfig, SandboxHandle, etc.)
├── host-hosted-api/     DELETED        │
├── sandbox/             MERGED         │
│                                       │
├── core/                KEPT           ├── core/               KEPT (combinators, ResourceRef, etc.)
├── orchestration/       KEPT           ├── orchestration/      KEPT (Orchestrator, whileLoopOrchestrator)
├── sandbox-local/       KEPT           ├── sandbox-local/      KEPT (Node subprocess for tools)
├── catalog.ts           KEPT           ├── catalog.ts          KEPT
├── acp.ts               KEPT           ├── acp.ts              KEPT
├── acp-core.ts          KEPT           ├── acp-core.ts         KEPT
├── acp.browser.ts       KEPT           ├── acp.browser.ts      KEPT
├── topology.ts          KEPT           ├── topology.ts         KEPT
└── index.ts             UPDATED        └── index.ts            UPDATED

packages/state/src/                     packages/state/src/
└── (unchanged)                         └── (unchanged)
```

`@fireline/state` is untouched. It already does the right thing: stream-db + TanStack DB live queries over the durable stream. The redesign just makes the composition explicit: `handle.state.url` → `createFirelineDB({ stateStreamUrl })` → `useLiveQuery(...)`.

---

## 7. Migration plan

### M1 — Ship the `Sandbox` class alongside old surface (1 day)

Add `sandbox.ts`, `admin.ts`, `types.ts`. Export via `@fireline/client/v2`. The `Sandbox` class targets the existing `/v1/runtimes` endpoints with field mapping. Zero server changes.

### M2 — Rewire browser harness (half day)

Replace `createFirelineHost` with `new Sandbox(...)`. The harness's `SessionHarness` component uses `sandbox.provision()` for launch, `sandbox.admin.destroy()` for cleanup, and `@agentclientprotocol/sdk` directly for ACP sessions (which it already does today — the only change is where the ACP URL comes from: `handle.acp.url` instead of a hardcoded proxy constant).

### M3 — Update tests (1 day)

Rewrite `host.test.ts` to use the `Sandbox` class. Merge `host-hosted-api.test.ts` (same class, different URL). Add `sandbox.test.ts` (unit, mock fetch) and `sandbox-integration.test.ts` (integration, real binaries).

### M4 — Delete old surface (half day)

Move v2 exports to the package root. Delete `host.ts`, `host/`, `host-fireline/`, `host-hosted-api/`, `sandbox/` (old). Update `index.ts`.

### M5 — Server-side endpoint rename `/v1/runtimes` → `/v1/sandboxes` (separate)

One-line path constant change in `sandbox.ts` after the Rust rename.

**Total: ~3 days.**

---

## 8. Composition with other proposals

| Proposal | How it composes with the client API |
|---|---|
| **Cross-host discovery** ([`./cross-host-discovery.md`](./cross-host-discovery.md)) | A `RemoteApiProvider` reads the `hosts:tenant-<id>` stream to discover sandbox servers. The client doesn't change — `new Sandbox({ serverUrl: discoveredUrl })` works against any server. |
| **Resource discovery** ([`./resource-discovery.md`](./resource-discovery.md)) | `SandboxConfig.resources` carries `DurableStreamBlob` and `StreamFs` refs. The server's provider resolves them at provision time. The client sends declarative intent; the server does the fetching. |
| **Secrets injection** ([`./deployment-and-remote-handoff.md`](./deployment-and-remote-handoff.md) §5) | `SandboxConfig.envVars` carries secret values. A future `SecretsInjectionComponent` (a Harness combinator) strips real secrets and replaces them with placeholders at the conductor layer. The client sends secrets in the config; the combinator decides what the agent actually sees. |
| **Stream-FS** ([`./stream-fs-spike.md`](./stream-fs-spike.md)) | `ResourceRef { kind: 'streamFs', source_ref, revision, mode }` in `SandboxConfig.resources`. The server's `DurableStreamMounter` resolves the snapshot. From the client's perspective, it's just another `ResourceRef` variant. |
| **Sandbox provider model** ([`./sandbox-provider-model.md`](./sandbox-provider-model.md)) | The `Sandbox` class is the TS client for the Rust `ProviderDispatcher`. `provision()` → `POST /v1/sandboxes` → `ProviderDispatcher::provision()` → whichever `SandboxProvider` impl (Local, Docker, Microsandbox, Remote) the server is configured with. Provider selection is server-side, invisible to the client. |
| **TLA verification** (`verification/spec/`) | The primitives the client exposes are the ones the TLA spec checks. `sandbox.provision()` exercises `ProvisionReturnsReachableRuntime`. ACP `connection.newSession()` exercises `SessionDurableAcrossRuntimeDeath`. The stream observation layer exercises `SessionAppendOnly`. The client API is the user-facing projection of formally-verified invariants. |

---

## Appendix: the operator surface — `SandboxAdmin`

The primitive is two methods. Operators, dev tools, and dashboards need more. Those live on `SandboxAdmin`:

```typescript
interface SandboxAdmin {
  get(id: string): Promise<SandboxDescriptor | null>
  list(labels?: Record<string, string>): Promise<SandboxDescriptor[]>
  findOrCreate(config: SandboxConfig): Promise<SandboxHandle>
  destroy(id: string): Promise<void>
  status(id: string): Promise<SandboxStatus>
  executeDetailed(id: string, command: string, opts?: ExecuteOptions): Promise<ExecutionResult>
  healthCheck(): Promise<boolean>
}

// Accessed via sandbox.admin
const sandbox = new Sandbox({ serverUrl })
await sandbox.admin.destroy(handle.id)
const all = await sandbox.admin.list({ team: 'frontend' })
```

This is a separate interface, not the primitive. The browser harness uses it. The CLI uses it. Agent code does not.

## Appendix: why `@fireline/state` stays separate

`@fireline/state` is a **read-only reactive view** over a durable stream. It has zero knowledge of sandboxes, provision, or lifecycle. Its input is a URL; its output is TanStack DB collections. That's a fundamentally different concern from `@fireline/client` (which sends HTTP requests to a server).

Merging them would force every state-observation consumer to pull in the sandbox lifecycle code, and every sandbox-lifecycle caller to pull in TanStack DB. Neither dependency makes sense for the other's use case. The two packages compose through `SandboxHandle.state.url` — the control plane gives you the endpoint, the data plane subscribes to it.

This is the same split as Ramp Inspect's architecture: *"the API creates sessions; a separate Durable Object holds the state; clients observe the state reactively."* In our case: `@fireline/client` creates sandboxes; `@fireline/state` observes the stream. The handle is the join point.
