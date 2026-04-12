# Client API Redesign — Sandbox as Anthropic Primitive

> **Status:** architectural proposal (revised — narrowed to match the Anthropic primitive table)
> **Replaces:** the current multi-layer TS client in `packages/client/src/` (Host + Sandbox + Orchestrator + multiple satisfier modules)
> **Companion:** [`./sandbox-provider-model.md`](./sandbox-provider-model.md) — the Rust-side provider model
> **Related:**
> - [`./client-primitives.md`](./client-primitives.md) — the v2 design this proposal supersedes at the Host/Sandbox boundary
> - [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md) — the Anthropic primitive table (Session, Orchestration, Harness, Sandbox, Resources, Tools)
> - Current TS client: `packages/client/src/` (~15 modules, 40+ exported types)

---

## 1. TL;DR — two methods, matching the Anthropic primitive table

Anthropic's managed-agents post defines the Sandbox primitive as:

```
provision({resources}) → execute(name, input) → String
```

*"Any executor configured once and called many times as a tool."*

The client surface matches:

```typescript
import { Sandbox } from '@fireline/client'

const sandbox = new Sandbox({ serverUrl: 'http://localhost:4440' })

// provision — hand me a place where an agent can run
const handle = await sandbox.provision({
  name: 'my-agent',
  agentCommand: ['npx', '-y', '@anthropic-ai/claude-agent-sdk'],
  topology: topology(durableTrace(), approvalGate({ scope: 'tool_calls' })),
})

// execute — run a command inside it
const result = await sandbox.execute(handle, 'ls -la /workspace')
console.log(result) // stdout string

// ACP sessions — SEPARATE PLANE, not on Sandbox
import { ClientSideConnection, PROTOCOL_VERSION } from '@agentclientprotocol/sdk'
const ws = new WebSocket(handle.acp.url)
const connection = new ClientSideConnection(handler, createWebSocketStream(ws))
await connection.initialize({ protocolVersion: PROTOCOL_VERSION, clientInfo: { name: 'my-app', version: '0.0.1' }, clientCapabilities: { fs: { readTextFile: false } } })
const { sessionId } = await connection.newSession({ cwd: '/' })
await connection.prompt({ sessionId, prompt: [{ type: 'text', text: 'Hello' }] })

// State observation — SEPARATE PLANE, not on Sandbox
import { createFirelineDB } from '@fireline/state'
const db = createFirelineDB({ stateStreamUrl: handle.state.url })
```

**Two methods.** `provision` and `execute`. Everything else is orthogonal:

- **Sessions** → ACP plane via `handle.acp` (`ClientSideConnection` from `@agentclientprotocol/sdk`)
- **State observation** → durable stream via `handle.state` (`@fireline/state`)
- **Orchestration** → `wake(session_id)` is a separate primitive, not on Sandbox
- **Listing / finding / admin** → operator concern, not the primitive interface (see §3)

---

## 2. The Sandbox primitive

```typescript
/**
 * The Sandbox primitive from Anthropic's managed-agent taxonomy.
 *
 * "Any executor configured once and called many times as a tool."
 *
 * Two methods: provision (create a sandbox) and execute (run a
 * command inside it). The returned SandboxHandle carries ACP and
 * state-stream endpoints so callers can reach the ACP plane (for
 * sessions) and the state plane (for durable stream observation)
 * without the Sandbox primitive knowing about either.
 *
 * This class is a thin HTTP client. It holds a server URL and
 * sends requests. It does not hold sandbox state, track sessions,
 * or manage connections. It is the primitive, not the management
 * surface.
 */
class Sandbox {
  constructor(opts: SandboxOptions)

  /**
   * Provision a sandbox — hand me a place where an agent can run.
   *
   * Creates a sandbox on the server, waits for it to become ready,
   * and returns a handle carrying the ACP and state-stream endpoints.
   */
  provision(config: SandboxConfig): Promise<SandboxHandle>

  /**
   * Execute a command inside a provisioned sandbox.
   *
   * Provider-level exec — bypasses ACP. Useful for setup, health
   * checks, and tool calls. Returns the stdout of the command.
   */
  execute(handle: SandboxHandle, input: string): Promise<string>
}

interface SandboxOptions {
  /** Base URL of the Fireline server. */
  readonly serverUrl: string

  /** Optional bearer token. */
  readonly token?: string

  /** Startup timeout for polling sandbox readiness (default: 20s). */
  readonly startupTimeoutMs?: number
}

interface SandboxHandle {
  /** Unique sandbox identifier. */
  readonly id: string

  /** ACP WebSocket endpoint. Open a ClientSideConnection here for sessions. */
  readonly acp: Endpoint

  /** Durable state stream endpoint. Subscribe with @fireline/state. */
  readonly state: Endpoint
}

interface SandboxConfig {
  /** Human-readable name. */
  name?: string

  /** Agent binary + args to run inside the sandbox. */
  agentCommand?: readonly string[]

  /** OCI image for container/VM providers. Ignored by local subprocess. */
  image?: string

  /** Topology (combinator chain) for the conductor. */
  topology?: Topology

  /** Resources to mount inside the sandbox. */
  resources?: readonly ResourceRef[]

  /** Environment variables visible to the agent process. */
  envVars?: Readonly<Record<string, string>>

  /** Labels for operator-level lookup. Not used by the primitive. */
  labels?: Readonly<Record<string, string>>

  /** Explicit state stream name. Auto-generated if omitted. */
  stateStream?: string

  /** Provider hint. Auto-select if omitted. */
  provider?: string
}
```

**That's the entire primitive surface.** Two methods, three types. The `Sandbox` class, `SandboxHandle`, and `SandboxConfig`.

**Why `execute` returns `string`, not `ExecutionResult`:** the Anthropic primitive definition is `execute(name, input) → String`. The primitive is "call a tool, get a string back." A richer `ExecutionResult` with `exitCode`, `stderr`, `durationMs`, `timedOut` is an operator extension (see §3) — useful, but not the primitive contract. If a caller needs structured results, they use the operator extension's `executeDetailed()` method.

**Why `SandboxHandle` carries `acp` + `state` endpoints:** these are how the caller reaches the two other planes (ACP for sessions, durable-streams for state) without the Sandbox primitive knowing about either. The Sandbox provisions a runtime; the handle tells you where to find it. No side channels. No leaky abstractions.

---

## 3. Operator extensions — `SandboxAdmin`

The primitive is two methods. But the browser harness, CLI tools, and operator dashboards need more: listing, finding, destroying, status-checking, health-probing. Those are **operator concerns**, not primitive concerns. They live on a separate interface:

```typescript
/**
 * Operator extensions for sandbox lifecycle management.
 *
 * NOT part of the Anthropic primitive surface. These methods are
 * for dev tools, browser harnesses, operator CLIs, and admin
 * dashboards. They hit the same server but expose the management
 * surface that the primitive deliberately excludes.
 */
interface SandboxAdmin {
  /** Look up a sandbox by id. */
  get(id: string): Promise<SandboxDescriptor | null>

  /** List sandboxes, optionally filtered by labels. */
  list(labels?: Readonly<Record<string, string>>): Promise<SandboxDescriptor[]>

  /** Find a sandbox by labels, or provision one if none match. */
  findOrCreate(config: SandboxConfig): Promise<SandboxHandle>

  /** Destroy a sandbox. Idempotent. */
  destroy(id: string): Promise<void>

  /** Current lifecycle status. */
  status(id: string): Promise<SandboxStatus>

  /** Execute with full structured result (exitCode, stderr, etc.). */
  executeDetailed(id: string, command: string, opts?: ExecuteOptions): Promise<ExecutionResult>

  /** Server-level health check. */
  healthCheck(): Promise<boolean>
}

type SandboxStatus = 'creating' | 'ready' | 'busy' | 'idle' | 'stopped' | 'broken'

interface SandboxDescriptor {
  readonly id: string
  readonly provider: string
  readonly status: SandboxStatus
  readonly acp: Endpoint
  readonly state: Endpoint
  readonly labels: Readonly<Record<string, string>>
  readonly createdAtMs: number
  readonly updatedAtMs: number
}

interface ExecuteOptions {
  readonly timeout?: number
  readonly env?: Readonly<Record<string, string>>
}

interface ExecutionResult {
  readonly exitCode: number
  readonly stdout: string
  readonly stderr: string
  readonly durationMs: number
  readonly timedOut: boolean
}
```

The `Sandbox` class can optionally expose an `admin` accessor for callers that need management:

```typescript
const sandbox = new Sandbox({ serverUrl: 'http://localhost:4440' })
const handle = await sandbox.provision(config)

// Primitive path: execute via the primitive
const output = await sandbox.execute(handle, 'echo hello')

// Operator path: structured execute, status, destroy
const result = await sandbox.admin.executeDetailed(handle.id, 'echo hello')
const status = await sandbox.admin.status(handle.id)
await sandbox.admin.destroy(handle.id)
```

This is the same split as `fs` in Node: `fs.readFile()` is the primitive; `fs.stat()`, `fs.readdir()`, `fs.chmod()` are operator extensions. Both exist; they're separated by intent.

---

## 4. HTTP API mapping

| Method | HTTP | Path | Body | Returns |
|---|---|---|---|---|
| **Primitive** | | | | |
| `sandbox.provision(config)` | `POST` | `/v1/sandboxes` | `SandboxConfig` JSON | `SandboxDescriptor` → mapped to `SandboxHandle` |
| `sandbox.execute(handle, input)` | `POST` | `/v1/sandboxes/{id}/execute` | `{ command: input }` | `{ stdout: string }` |
| **Operator extensions** | | | | |
| `admin.get(id)` | `GET` | `/v1/sandboxes/{id}` | — | `SandboxDescriptor` |
| `admin.list(labels)` | `GET` | `/v1/sandboxes?label.k=v` | — | `SandboxDescriptor[]` |
| `admin.findOrCreate(config)` | `POST` | `/v1/sandboxes/find-or-create` | `SandboxConfig` JSON | `SandboxDescriptor` |
| `admin.destroy(id)` | `DELETE` | `/v1/sandboxes/{id}` | — | `void` |
| `admin.status(id)` | `GET` | `/v1/sandboxes/{id}` | — | `.status` field |
| `admin.executeDetailed(id, cmd)` | `POST` | `/v1/sandboxes/{id}/execute` | `{ command, timeout?, env? }` | `ExecutionResult` |
| `admin.healthCheck()` | `GET` | `/healthz` | — | `boolean` |

**ACP sessions and state observation do NOT go through this HTTP API.** They use `handle.acp.url` (WebSocket) and `handle.state.url` (HTTP+SSE) directly.

---

## 5. What gets deleted from `packages/client/`

| Current module | Disposition |
|---|---|
| `host.ts` (binary spawner) | **DELETED** |
| `host/index.ts` (`Host` interface) | **DELETED** — `Host.provision` → `Sandbox.provision`; `Host.wake` → separate Orchestration primitive; `Host.status` → `SandboxAdmin.status`; `Host.stop` → `SandboxAdmin.destroy` |
| `host-fireline/client.ts` | **REPLACED** by `Sandbox` class (same HTTP calls, narrower interface) |
| `host-fireline/orchestrator.ts` | Orchestrator moves to standalone export (unchanged) |
| `host-hosted-api/client.ts` | **DELETED** — `Sandbox` class works against any server URL |
| `sandbox/index.ts` (old tool-execution interface) | **REPLACED** by `Sandbox.execute` |
| `sandbox-local/` | **KEPT** as Node-only convenience |
| `orchestration/` | **KEPT** — orchestration is a separate primitive |
| `core/` | **KEPT** — pure types |
| `catalog.ts` | **KEPT** |
| `acp.ts` / `acp-core.ts` | **KEPT** — consumed by callers directly, not wrapped by Sandbox |
| `topology.ts` | **KEPT** |

**The `Sandbox` class replaces 4 modules with 1.** Net: ~800 lines deleted, ~150 lines added.

---

## 6. What the browser-harness changes to

**Before (current):**
```typescript
const host: Host = createFirelineHost({ controlPlaneUrl, sharedStateUrl })
const handle = await host.provision({ agentCommand, metadata: { name: 'harness' } })
const status = await host.status(handle)
const outcome = await host.wake(handle)
await host.stop(handle)

const ws = new WebSocket('ws://localhost:5173/acp') // hardcoded proxy URL
```

**After:**
```typescript
const sandbox = new Sandbox({ serverUrl: '/cp' })
const handle = await sandbox.provision({ name: 'harness', agentCommand })

// ACP — from the handle, not hardcoded
const ws = new WebSocket(handle.acp.url)
const connection = new ClientSideConnection(handler, createWebSocketStream(ws))
await connection.initialize(...)
const { sessionId } = await connection.newSession({ cwd: '/' })

// Operator extensions for the harness UI
const status = await sandbox.admin.status(handle.id)
await sandbox.admin.destroy(handle.id)
```

The browser harness is a dev tool — it uses `sandbox.admin` for status/destroy. But its core flow (provision → connect ACP → prompt) uses only the two primitive methods plus the ACP SDK directly.

---

## 7. Comparison to Cased

| Dimension | Cased | Fireline |
|---|---|---|
| **Primitive surface** | `create / execute / destroy` (3 methods) | `provision / execute` (2 methods). `destroy` is an operator extension, not the primitive. |
| **Handle** | `Sandbox` object with id + state + labels | `SandboxHandle` with id + `acp` endpoint + `state` endpoint |
| **Execution** | `sandbox.execute(cmd) → ExecutionResult` | `sandbox.execute(handle, input) → string` (primitive); `admin.executeDetailed(id, cmd) → ExecutionResult` (operator) |
| **Sessions** | Not modeled | ACP plane via `handle.acp.url` — deliberate non-wrapping |
| **Management** | On the `Sandbox` class: `find`, `get_or_create`, `destroy` | On `SandboxAdmin`: `get`, `list`, `findOrCreate`, `destroy`, `status` — separated from the primitive |
| **Provider dispatch** | Client-side `SandboxManager` | Server-side `ProviderDispatcher` — client is provider-agnostic |
| **Pooling** | Client-side `SandboxPool` | Server-side `PooledProvider`; client uses `findOrCreate` |

**What we adopt:** the `provision`/`execute` pattern, the handle-as-return-value shape, label-based lookup, provider-agnostic client.

**What we narrow:** Cased puts management methods (destroy, find, get_or_create) on the primary `Sandbox` class. We separate them into `SandboxAdmin` because the Anthropic primitive table says Sandbox is `provision + execute`, not `provision + execute + destroy + find + status`. The operator surface exists — it's just not the primitive.

---

## 8. Migration plan

### Phase M1 — Ship the `Sandbox` class alongside the old surface (1 day)

Add `packages/client/src/sandbox-v2.ts` exporting `Sandbox` class + `SandboxAdmin` + types. Export via `@fireline/client/v2`:

```typescript
import { Sandbox } from '@fireline/client/v2'
```

The `Sandbox` class targets the existing `/v1/runtimes` endpoints (renamed `/v1/sandboxes` is a server-side change that happens later). Field mapping is a thin adapter: `POST /v1/runtimes` body stays the same, response `HostDescriptor` is mapped to `SandboxHandle`.

**Zero server changes.** The v2 client is a 150-line adapter over the existing API.

### Phase M2 — Rewire the browser harness (half day)

`packages/browser-harness/src/app.tsx` switches from `createFirelineHost` to `new Sandbox(...)`. The harness uses `sandbox.provision()` for launch, `sandbox.admin.status()` / `sandbox.admin.destroy()` for the control buttons, and `new ClientSideConnection(...)` with `handle.acp.url` for the ACP session.

### Phase M3 — Update tests (1 day)

| Test file | Change |
|---|---|
| `test/host.test.ts` | Rewrite: `new Sandbox(...)` → `sandbox.provision()` → `sandbox.execute()` → `sandbox.admin.destroy()` |
| `test/host-hosted-api.test.ts` | Merge into `host.test.ts` — `new Sandbox({ serverUrl: remote })` works against any server |
| `test/sandbox-local.test.ts` | Unchanged |
| `test/catalog.test.ts` | Unchanged |
| `test/topology.test.ts` | Unchanged |
| `test/acp.test.ts` | Unchanged |
| `test/tier5-smoke.browser.test.ts` | Update to match M2's new harness shape |

### Phase M4 — Delete the v1 surface (half day)

Move `sandbox-v2.ts` to `sandbox-client.ts` (or `client.ts`). Delete: `host.ts`, `host/`, `host-fireline/`, `host-hosted-api/`, `sandbox/` (old tool-execution interface). Update `index.ts` exports. Remove `/v2` subpath after one release.

### Phase M5 — Server-side endpoint rename (separate)

After the Rust-side provider model Phase P6 renames endpoints to `/v1/sandboxes`, the TS `Sandbox` class switches path constants. One-line change.

**Total: ~3 days across M1-M4.**

---

## 9. Test strategy

### New tests

1. **`test/sandbox-client.test.ts`** — unit tests for the `Sandbox` class. Mock fetch: `provision()` → `POST /v1/runtimes` returns descriptor, `execute()` → `POST /v1/runtimes/{id}/execute` returns stdout. Verify the `SandboxHandle` carries correct `acp` + `state` endpoints. ~50 lines.

2. **`test/sandbox-admin.test.ts`** — unit tests for operator extensions. Mock fetch: `get`, `list`, `destroy`, `status`, `executeDetailed`, `healthCheck`. ~80 lines.

3. **`test/sandbox-integration.test.ts`** — integration test replacing `host.test.ts`. Spawns real binaries. Full lifecycle: `sandbox.provision()` → `sandbox.execute('echo hello')` → `new ClientSideConnection(handle.acp.url)` → `connection.newSession()` → `connection.prompt(...)` → `sandbox.admin.destroy(handle.id)`. ~120 lines.

### What this proves

- `sandbox-client.test.ts` → primitive works against mock (fast, no cargo)
- `sandbox-admin.test.ts` → operator extensions work (fast, no cargo)
- `sandbox-integration.test.ts` → full stack end-to-end (slow, needs cargo build)

Every test except the integration test runs without Rust binaries. This is a material improvement: today `test/host.test.ts` requires `cargo build` in `beforeAll`.

---

## Appendix: type mapping — old → new

| Old type | New type | Notes |
|---|---|---|
| `Host` interface (4 methods) | `Sandbox` class (2 methods) | `provision` stays; `wake/status/stop` move to Orchestration or SandboxAdmin |
| `HostHandle` | `SandboxHandle` | Same fields minus `kind` (provider-agnostic at the client) |
| `HostStatus` (discriminated union) | `SandboxStatus` (string enum on admin) | Slimmed — the primitive doesn't expose status |
| `WakeOutcome` | deleted | Wake is Orchestration, not Sandbox |
| `ProvisionSpec` | `SandboxConfig` | Renamed, same fields |
| `FirelineHostOptions` | `SandboxOptions` | `controlPlaneUrl` → `serverUrl`; `sharedStateUrl` → deleted (server-side) |
| `FirelineClient` (proposed in prior revision) | deleted | Was a management surface; management moved to `SandboxAdmin` |
| `createFirelineHost()` | `new Sandbox(...)` | Class, not factory function |
| `createHostedApiHost()` | `new Sandbox({ serverUrl: remoteUrl })` | Same class, different URL |
| `Sandbox` interface (old, tool execution) | merged into `Sandbox.execute()` | Structured tool calls → `admin.executeDetailed()` |

## Appendix: module layout — old → new

```
packages/client/src/                    packages/client/src/
├── host.ts              DELETED        ├── sandbox-client.ts   NEW (Sandbox class + SandboxAdmin)
├── host/                DELETED        ├── sandbox-types.ts    NEW (SandboxConfig, SandboxHandle, etc.)
├── host-fireline/       DELETED        │
├── host-hosted-api/     DELETED        │
├── sandbox/             MERGED         │
│                                       │
├── core/                KEPT           ├── core/               KEPT
├── orchestration/       KEPT           ├── orchestration/      KEPT
├── sandbox-local/       KEPT           ├── sandbox-local/      KEPT
├── catalog.ts           KEPT           ├── catalog.ts          KEPT
├── acp.ts               KEPT           ├── acp.ts              KEPT
├── acp-core.ts          KEPT           ├── acp-core.ts         KEPT
├── acp.browser.ts       KEPT           ├── acp.browser.ts      KEPT
├── browser.ts           KEPT           ├── browser.ts          KEPT
├── topology.ts          KEPT           ├── topology.ts         KEPT
└── index.ts             UPDATED        └── index.ts            UPDATED
```

## Appendix: why two methods, not three

Cased's `Sandbox` has three core methods: `create`, `execute`, `destroy`. The Anthropic primitive table has two: `provision`, `execute`. The difference is `destroy`.

We follow Anthropic: `destroy` is an operator concern, not a primitive concern. The primitive's contract is *"configure once, call many times."* Cleanup is the operator's job — they decide when a sandbox is no longer needed, they call `admin.destroy()`, and the server handles teardown. The primitive user shouldn't have to think about lifecycle cleanup; that's what the Orchestration primitive and the operator surface are for.

If the user forgets to destroy, the server's idle-timeout or TTL policy handles it. The primitive is stateless; the management surface is not. Keeping them separate is the whole point.
