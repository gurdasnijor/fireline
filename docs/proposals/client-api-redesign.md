# Client API Redesign — Unified Sandbox Surface

> **Status:** architectural proposal
> **Replaces:** the current multi-layer TS client in `packages/client/src/` (Host + Sandbox + Orchestrator + multiple satisfier modules)
> **Inspired by:** [Cased sandboxes](https://github.com/cased/sandboxes) client surface (Python `Sandbox.create()` → `sandbox.execute()` → `sandbox.destroy()`)
> **Companion:** [`./sandbox-provider-model.md`](./sandbox-provider-model.md) — the Rust-side provider model this TS client maps onto
> **Related:**
> - [`./client-primitives.md`](./client-primitives.md) — the v2 design this proposal supersedes at the Host/Sandbox boundary
> - [`./deployment-and-remote-handoff.md`](./deployment-and-remote-handoff.md) — deployment topology
> - Current TS client: `packages/client/src/` (~15 modules, 40+ exported types)

---

## 1. TL;DR — one Sandbox, provider-agnostic

The user writes:

```typescript
import { createFirelineClient } from '@fireline/client'

const client = createFirelineClient({ serverUrl: 'http://localhost:4440' })

const sandbox = await client.create({
  name: 'my-agent',
  agentCommand: ['npx', '-y', '@anthropic-ai/claude-agent-sdk'],
  topology: topology(durableTrace(), approvalGate({ scope: 'tool_calls' })),
  resources: [{ source_ref: { kind: 'localPath', host_id: 'self', path: '~/project' }, mount_path: '/workspace' }],
})

// Sandbox carries its own ACP + state endpoints — no separate Host/Sandbox dance
const session = await sandbox.newSession({ cwd: '/workspace' })
const response = await session.prompt('What files are in this directory?')
console.log(response)

// Or execute a command directly (bypasses ACP, provider-level exec)
const result = await sandbox.execute('ls -la /workspace')
console.log(result.stdout)

await sandbox.destroy()
```

**No separate Host/Sandbox distinction at the client level.** The old `host.provision()` → `host.wake()` → `host.status()` → `host.stop()` four-verb dance is replaced by `client.create()` → `sandbox.execute()` / `sandbox.newSession()` → `sandbox.destroy()`. The `Host` concept was always "the thing that hands you a running agent" — the `Sandbox` IS that thing. ACP sessions live inside sandboxes.

This matches Cased's model:
```python
# Cased (Python):
sandbox = await Sandbox.create(provider='e2b', image='python')
result = await sandbox.execute('print("hello")')
await sandbox.destroy()
```

Same pattern, different substrate: Fireline's sandboxes carry ACP + durable-streams integration; Cased's don't.

---

## 2. The Sandbox handle

```typescript
interface Sandbox {
  /** Unique sandbox identifier (matches Rust SandboxHandle.id) */
  readonly id: string

  /** Which provider is running this sandbox ('local', 'docker', 'microsandbox', 'remote') */
  readonly provider: string

  /** ACP WebSocket endpoint. Open a ClientSideConnection here. */
  readonly acp: Endpoint

  /** Durable state stream endpoint. Subscribe with @fireline/state. */
  readonly state: Endpoint

  /** Current lifecycle status. */
  status(): Promise<SandboxStatus>

  /**
   * Execute a command inside the sandbox. Provider-level exec —
   * bypasses ACP. Useful for setup, health checks, tool calls.
   */
  execute(command: string, opts?: ExecuteOptions): Promise<ExecutionResult>

  /** Destroy the sandbox. Idempotent. */
  destroy(): Promise<void>

  // --- ACP session management ---

  /**
   * Open a new ACP session inside this sandbox.
   * Connects to sandbox.acp.url, initializes, calls session/new.
   */
  newSession(opts?: NewSessionOptions): Promise<AcpSession>

  /**
   * Reconnect to an existing ACP session inside this sandbox.
   * Connects to sandbox.acp.url, initializes, calls session/load.
   */
  loadSession(sessionId: string, opts?: LoadSessionOptions): Promise<AcpSession>
}

interface AcpSession {
  readonly sessionId: string
  prompt(text: string): Promise<PromptResponse>
  disconnect(): Promise<void>
}

type SandboxStatus =
  | { readonly kind: 'creating' }
  | { readonly kind: 'ready' }
  | { readonly kind: 'busy' }
  | { readonly kind: 'idle' }
  | { readonly kind: 'stopped' }
  | { readonly kind: 'broken'; readonly message: string }

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

interface NewSessionOptions {
  readonly cwd?: string
  readonly mcpServers?: readonly unknown[]
}

interface LoadSessionOptions {
  readonly cwd?: string
  readonly mcpServers?: readonly unknown[]
}

interface PromptResponse {
  readonly stopReason: string
  readonly messages: readonly unknown[]
}
```

**Key design choice: `Sandbox` is a live object, not a stateless handle.** Cased's `Sandbox` class holds a reference to its provider and delegates `execute()` / `destroy()` through it. We do the same: the `Sandbox` object returned by `client.create()` holds the server URL and sandbox id internally, and every method is a single HTTP call to the server.

**Why `newSession()` and `loadSession()` live on `Sandbox`, not on `FirelineClient`:** sessions are ACP concepts that live *inside* a running sandbox. The sandbox's `acp.url` is the WebSocket endpoint; the session is opened over that connection. Making sessions a sandbox-level concern (not a client-level concern) matches the Rust-side model where `SandboxProvider::create()` returns endpoints and the provider has no opinion about what ACP sessions happen inside.

---

## 3. The FirelineClient surface

```typescript
interface FirelineClient {
  /** Provision a new sandbox. */
  create(config: SandboxConfig): Promise<Sandbox>

  /** Look up a sandbox by id. Returns null if not found. */
  get(id: string): Promise<Sandbox | null>

  /** List sandboxes, optionally filtered by labels. */
  list(labels?: Readonly<Record<string, string>>): Promise<Sandbox[]>

  /** Find the first sandbox matching labels, or create one if none match. */
  findOrCreate(config: SandboxConfig): Promise<Sandbox>

  /** Server-level health check. */
  healthCheck(): Promise<boolean>

  /** Clean up any client-side resources (WebSocket pools, etc.). */
  close(): Promise<void>
}

interface FirelineClientOptions {
  /** Base URL of the Fireline host HTTP server. */
  readonly serverUrl: string

  /** Optional bearer token for authenticated endpoints. */
  readonly token?: string

  /** Startup timeout for polling sandbox readiness (default: 20s). */
  readonly startupTimeoutMs?: number

  /** Poll interval for readiness checks (default: 100ms). */
  readonly pollIntervalMs?: number
}

function createFirelineClient(opts: FirelineClientOptions): FirelineClient
```

**One client, one server URL, any provider.** The old model had `createFirelineHost` (for the Fireline control plane), `createHostedApiHost` (for a remote hosted API), plus the legacy `createHostClient` (Node binary spawner). All three are replaced by `createFirelineClient` pointed at any Fireline server. The server handles provider dispatch internally per [`sandbox-provider-model.md`](./sandbox-provider-model.md) §4.

**`findOrCreate(config)` is the pooling / reuse surface.** Cased implements this as `get_or_create()` — search by labels first, create if not found. Same pattern here: the server checks existing sandboxes matching `config.labels`, reuses if found, creates if not. This is the building block for connection pooling, dev-mode reuse, and multi-tenant sandbox sharing.

---

## 4. SandboxConfig — what the user passes

```typescript
interface SandboxConfig {
  /** Human-readable name. Used for logging and default stream names. */
  name?: string

  /** The agent binary + args to run inside the sandbox. */
  agentCommand?: readonly string[]

  /** OCI image for container/VM providers (docker, microsandbox). Ignored by local. */
  image?: string

  /** Topology (combinator chain) for the conductor inside the sandbox. */
  topology?: Topology

  /** Resources to mount inside the sandbox. */
  resources?: readonly ResourceRef[]

  /** Environment variables visible to the agent process. */
  envVars?: Readonly<Record<string, string>>

  /** Labels for sandbox lookup, filtering, and pool reuse. */
  labels?: Readonly<Record<string, string>>

  /** Explicit state stream name. Auto-generated from sandbox id if omitted. */
  stateStream?: string

  /** Provider hint. Auto-select if omitted. */
  provider?: string
}
```

This maps directly to the Rust `SandboxConfig` in [`sandbox-provider-model.md`](./sandbox-provider-model.md) §2. The only difference is `durable_streams_url` — in the TS client that's a server-side concern (the server knows its own durable-streams URL), not a client-side config field. The client doesn't need to know where the streams live.

**Cased comparison:** Cased's `SandboxConfig` carries `image`, `language`, `memory_mb`, `cpu_cores`, `timeout_seconds`, `env_vars`, `labels`, `setup_commands`, `working_dir`. We carry the same shape minus the resource-constraint fields (those are provider-internal in Fireline) and plus `topology` + `resources` (which Cased doesn't model because they don't have ACP or structured resource mounting).

---

## 5. HTTP API mapping

Every `FirelineClient` and `Sandbox` method maps to a single HTTP call:

| Client method | HTTP | Path | Body | Returns |
|---|---|---|---|---|
| `client.create(config)` | `POST` | `/v1/sandboxes` | `SandboxConfig` JSON | `SandboxDescriptor` → wrapped as `Sandbox` |
| `client.get(id)` | `GET` | `/v1/sandboxes/{id}` | — | `SandboxDescriptor \| null` |
| `client.list(labels)` | `GET` | `/v1/sandboxes?label.env=prod&label.tier=1` | — | `SandboxDescriptor[]` |
| `client.findOrCreate(config)` | `POST` | `/v1/sandboxes/find-or-create` | `SandboxConfig` JSON | `SandboxDescriptor` |
| `client.healthCheck()` | `GET` | `/healthz` | — | `boolean` |
| `sandbox.status()` | `GET` | `/v1/sandboxes/{id}` | — | `SandboxDescriptor.status` |
| `sandbox.execute(cmd)` | `POST` | `/v1/sandboxes/{id}/execute` | `{ command, timeout?, env? }` | `ExecutionResult` |
| `sandbox.destroy()` | `DELETE` | `/v1/sandboxes/{id}` | — | `void` |

**ACP methods (`newSession`, `loadSession`, `prompt`) don't go through the HTTP API** — they open a WebSocket directly to `sandbox.acp.url` using `@agentclientprotocol/sdk`'s `ClientSideConnection`. The HTTP API provisions and manages sandboxes; ACP is the data plane.

**Backward compatibility:** during the transition, the server serves both `/v1/runtimes` (old) and `/v1/sandboxes` (new) endpoints. The old paths are aliases that map to the same `ProviderDispatcher` handlers. The TS client sends to `/v1/sandboxes`; legacy callers continue to work against `/v1/runtimes` until a deprecation period ends.

---

## 6. What gets deleted from `packages/client/`

| Current module | Disposition |
|---|---|
| `host.ts` | **DELETED** — the legacy binary-spawner client. Fully superseded by `createFirelineClient` + server-side provisioning. |
| `host/index.ts` (`Host` interface, `HostHandle`, `HostStatus`, `WakeOutcome`, `ProvisionSpec`) | **DELETED** — `Host.provision` is replaced by `FirelineClient.create`; `Host.wake` → `client.get(id)` or `sandbox.status()` (the Wake verb collapses into status-polling + sandbox-level lifecycle); `Host.stop` → `sandbox.destroy()`. |
| `host-fireline/client.ts` (`createFirelineHost`, `FirelineHostOptions`) | **REPLACED** by `createFirelineClient`. The HTTP calls are nearly identical — `POST /v1/runtimes` becomes `POST /v1/sandboxes`, the polling loop is the same, the `HostHandle` → `Sandbox` wrapping is the same. |
| `host-fireline/orchestrator.ts` (`createFirelineHostOrchestrator`, `createFirelineClient`) | **MERGED** — the orchestrator moves to a standalone `@fireline/client/orchestration` export that wraps any `FirelineClient`. |
| `host-fireline/registry.ts` (`createStreamSessionRegistry`) | **KEPT** — the session registry is orthogonal to the provider model; it reads from `@fireline/state` and is consumed by the orchestrator. |
| `host-hosted-api/client.ts` (`createHostedApiHost`) | **DELETED** — `createFirelineClient` pointed at the remote server's URL does the same thing. There's no "hosted API vs local" distinction in the client anymore; all servers expose the same `/v1/sandboxes` API. |
| `sandbox/index.ts` (`Sandbox` interface for tool execution) | **MERGED** — the tool-execution `Sandbox` interface (with `ToolCall` / `ToolResult`) merges into the unified `Sandbox` handle via the `execute()` method. Tool-specific structured calls become a higher-level wrapper over `execute()`, not a separate trait. |
| `sandbox-local/client.ts` (`createLocalSandbox`) | **KEPT as a Node-only convenience** — for tests and CLI tools that want to run a tool in a subprocess without provisioning a full sandbox. Not part of the main `FirelineClient` surface. |
| `orchestration/index.ts` (`Orchestrator`, `whileLoopOrchestrator`, `SessionRegistry`) | **KEPT** — orchestration is orthogonal. The `WakeHandler` now calls `client.get(id)` + `sandbox.status()` instead of `host.wake(handle)`. |
| `core/` (`Combinator`, `Topology`, `ResourceRef`, `CapabilityRef`, etc.) | **KEPT** — pure serializable types. Zero changes. |
| `catalog.ts` (`CatalogClient`) | **KEPT** — agent catalog is orthogonal to the provider model. |
| `acp.ts` / `acp-core.ts` / `acp.browser.ts` | **KEPT** — ACP connectivity is consumed by `Sandbox.newSession()` / `Sandbox.loadSession()` internally. |
| `topology.ts` (`TopologyBuilder`) | **KEPT** — pure composition helpers. |

**Net deletion:** `host.ts`, `host/index.ts`, `host-fireline/client.ts`, `host-fireline/orchestrator.ts`, `host-hosted-api/`. ~800-1000 lines removed. **Net addition:** `client.ts` (the new `FirelineClient` + `Sandbox` impl), ~300-400 lines. Net reduction: ~500-600 lines with a simpler mental model.

---

## 7. What the browser-harness changes to

**Before (current, Tier 5):**
```typescript
import { createFirelineHost } from '@fireline/client/host-fireline'
import type { Host, SessionHandle, SessionStatus } from '@fireline/client/host'

const host: Host = createFirelineHost({
  controlPlaneUrl: CONTROL_PLANE_URL,
  sharedStateUrl: STATE_PROXY_URL,
})

const handle = await host.provision({ agentCommand, metadata: { name: 'harness', stateStream } })
const status = await host.status(handle)
const outcome = await host.wake(handle)
await host.stop(handle)

// ACP: open WebSocket manually to a hardcoded proxy URL
const ws = new WebSocket('ws://localhost:5173/acp')
```

**After (new API):**
```typescript
import { createFirelineClient } from '@fireline/client'

const client = createFirelineClient({ serverUrl: '/cp' })

const sandbox = await client.create({
  name: 'browser-harness',
  agentCommand: resolvedAgentCommand,
  stateStream: STATE_STREAM_NAME,
})

// sandbox.acp.url gives the ACP WebSocket URL — no hardcoded proxy
const session = await sandbox.newSession({ cwd: '/' })
const response = await session.prompt('Hello from the browser harness')

// Status is on the sandbox, not a separate host.status(handle) call
const status = await sandbox.status()

// Wake collapses: the user clicks "Wake" → sandbox.status() returns the state;
// if stopped, client.create() with the same labels reuses or re-provisions.
// No separate wake() verb needed at the client level.

await sandbox.destroy()
```

The browser-harness app.tsx drops:
- `useMemo<Host>(() => createFirelineHost({...}), [])` → `useMemo(() => createFirelineClient({...}), [])`
- `host.provision(spec)` → `client.create(config)` — returns a `Sandbox` with `.acp.url`, not a `HostHandle` that needs a separate proxy URL
- `host.wake(handle)` → removed from the UI (or: `sandbox.status()` + conditional `client.create()` with the same labels if stopped)
- `host.stop(handle)` → `sandbox.destroy()`
- The hardcoded `ACP_PROXY_URL = 'ws://${window.location.host}/acp'` → `sandbox.acp.url` read from the handle (vite proxy still works, but the URL comes from the server response, not a client-side constant)

**This fixes the port-4437-pinning fragility** identified in the API surface audit — the sandbox handle carries the actual ACP endpoint, so the browser doesn't need to guess the port.

---

## 8. Comparison to Cased client API

### What we adopt

| Cased pattern | Fireline equivalent |
|---|---|
| `Sandbox.create(provider='e2b', image='python')` | `client.create({ provider: 'microsandbox', image: 'python', agentCommand: [...] })` |
| `sandbox.execute('ls')` → `ExecutionResult` | `sandbox.execute('ls')` → `ExecutionResult` |
| `sandbox.destroy()` | `sandbox.destroy()` |
| `Sandbox.find(labels={...})` | `client.list({ env: 'prod' })` or `client.findOrCreate(config)` |
| `Sandbox.get_or_create(labels={...})` | `client.findOrCreate(config)` |
| `sandbox.upload('local.txt', '/remote.txt')` | `sandbox.execute('cat > /remote.txt', { stdin: content })` or via ACP `fs/write_text_file` |
| `sandbox.stream('tail -f /var/log/app.log')` | Future: `sandbox.executeStream(cmd)` returning `AsyncIterator<string>` |
| `sandbox.state` (enum: running/stopped/...) | `sandbox.status()` → `SandboxStatus` (async, fetches from server) |
| `SandboxManager.create_sandbox(config, fallback_providers=[...])` | `client.create(config)` — failover is server-side (`ProviderDispatcher`) |

### What differs

| Dimension | Cased | Fireline |
|---|---|---|
| **ACP sessions** | Not modeled | `sandbox.newSession()` / `sandbox.loadSession()` / `session.prompt()` — the sandbox carries a live ACP data plane |
| **Topology / combinators** | Not modeled | `config.topology` carries a `Topology` (combinator chain) that the conductor inside the sandbox interprets |
| **Durable state** | No concept of state streams | `sandbox.state` is a durable-streams endpoint; the browser subscribes via `@fireline/state` and `useLiveQuery` |
| **Provider dispatch** | Client-side `SandboxManager` | Server-side `ProviderDispatcher` — the client is provider-agnostic, sends to one URL |
| **Status** | Synchronous property (`sandbox.state`) | Async method (`sandbox.status()`) — the client fetches from the server each time |
| **Auto-configure** | `Sandbox._auto_configure()` reads env vars to pick providers | Not needed — the server picks the provider based on its config |
| **File transfer** | `sandbox.upload()` / `sandbox.download()` | Via ACP `fs/*` methods or `sandbox.execute('cat ...')` — no separate file-transfer surface |
| **Pooling** | Client-side `SandboxPool` with acquire/release | Server-side `PooledProvider` — the client calls `findOrCreate` and the server handles pooling internally |
| **Context manager** | `async with Sandbox.create() as sb:` auto-destroys | Not modeled in TS (no `using` equivalent in current Node); caller calls `sandbox.destroy()` explicitly or registers a process exit handler |

---

## 9. Migration plan

### Phase M1 — Ship the new client alongside the old (1 day)

Add `packages/client/src/v2/` with `client.ts`, `sandbox.ts`, `types.ts`. Export via a new subpath `@fireline/client/v2`:

```typescript
import { createFirelineClient } from '@fireline/client/v2'
```

The old `@fireline/client` surface stays unchanged. Both coexist. The v2 client targets the existing `/v1/runtimes` endpoints (not `/v1/sandboxes` — that's a server-side rename that happens in a later phase per [`sandbox-provider-model.md`](./sandbox-provider-model.md) §7 Phase P6). Field mapping:

| v2 client call | HTTP path | Notes |
|---|---|---|
| `client.create(config)` | `POST /v1/runtimes` | Body: `{ name, agentCommand, topology, resources, stateStream }` (same as current `ProvisionRequest`) |
| `client.get(id)` | `GET /v1/runtimes/{id}` | Response: `HostDescriptor` mapped to `SandboxDescriptor` |
| `sandbox.destroy()` | `POST /v1/runtimes/{id}/stop` + `DELETE /v1/runtimes/{id}` | Two calls; collapsed to one `DELETE /v1/sandboxes/{id}` after P6 |

This means M1 ships with **zero server changes**. The v2 client is a thin adapter over the existing API.

### Phase M2 — Rewire the browser harness to v2 (half day)

`packages/browser-harness/src/app.tsx` switches from `createFirelineHost` to `createFirelineClient`. The `SessionHarness` component uses `sandbox.newSession()` instead of manually constructing a `ClientSideConnection`. The hardcoded `ACP_PROXY_URL` is replaced by `sandbox.acp.url`.

### Phase M3 — Update tests (1 day)

| Test file | Change |
|---|---|
| `test/host.test.ts` | Rewrite to use `createFirelineClient` → `client.create()` → `sandbox.destroy()`. The spawn-control-plane pattern stays; only the client calls change. |
| `test/host-hosted-api.test.ts` | Merge into `host.test.ts` — the v2 client works against both local and hosted API servers. |
| `test/sandbox-local.test.ts` | Unchanged (sandbox-local is a Node-only convenience, not part of the main client surface). |
| `test/catalog.test.ts` | Unchanged. |
| `test/topology.test.ts` | Unchanged. |
| `test/acp.test.ts` | Unchanged (ACP connectivity is consumed internally by `Sandbox.newSession()`). |
| Browser test (`test/tier5-smoke.browser.test.ts`) | Rewrite to match M2's new browser-harness shape. |

### Phase M4 — Delete the v1 surface (half day)

Once all callers have migrated to `@fireline/client/v2`:

1. Move `v2/` contents to the package root: `client.ts` → `src/client.ts`, `sandbox.ts` → `src/sandbox.ts`, `types.ts` → `src/types.ts`.
2. Delete: `src/host.ts`, `src/host/`, `src/host-fireline/`, `src/host-hosted-api/`, `src/sandbox/` (the old tool-execution interface).
3. Update `src/index.ts` to export from the new paths.
4. Update `package.json` exports — remove the `/host`, `/host-fireline`, `/host-hosted-api` subpath exports; add `/v2` as an alias for the root (transitional).
5. Delete the `/v2` subpath export after one release cycle.

### Phase M5 — Server-side endpoint rename (separate, after Rust P6)

After the Rust-side `sandbox-provider-model.md` Phase P6 renames the HTTP endpoints from `/v1/runtimes` to `/v1/sandboxes`, the TS client switches to the new paths. The old paths remain as aliases on the server during the deprecation window. This is a one-line change in the client's base URL / path constants.

**Total: ~3 days of focused work across M1-M4.** M5 is a follow-on that waits for the server rename.

---

## 10. Test strategy

### Existing test files and their fate

| File | Lines | Status | Notes |
|---|---|---|---|
| `test/host.test.ts` | ~190 | **Rewrite** (M3) | Spawns control plane + fireline binary; exercises create/get/list/stop/delete. The test shape stays (it's an integration test against real binaries); only the client calls change from `host.create()` → `client.create()`, `host.stop()` → `sandbox.destroy()`. |
| `test/host-hosted-api.test.ts` | ~120 | **Merge into host.test.ts** (M3) | The v2 client is provider-agnostic; this test's fixture (mock HTTP server) becomes a second `describe` block in host.test.ts. |
| `test/sandbox-local.test.ts` | ~80 | **Unchanged** | Tests the Node-only subprocess sandbox for tool execution. Not part of the main client surface. |
| `test/catalog.test.ts` | ~60 | **Unchanged** | Agent catalog is orthogonal. |
| `test/topology.test.ts` | ~80 | **Unchanged** | Pure combinator composition tests. |
| `test/acp.test.ts` | ~40 | **Unchanged** | ACP connectivity. May gain a new test for `Sandbox.newSession()` that uses a mock ACP server. |
| `test/tier5-smoke.browser.test.ts` | ~230 | **Rewrite** (M3) | Playwright browser test exercising the harness UI. Mock fetch handlers update from `/cp/v1/runtimes` to `/cp/v1/sandboxes` (or `/cp/v1/runtimes` during M1-M3 while the server hasn't renamed yet). Button labels stay the same. |

### New tests to add

1. **`test/client.test.ts`** — unit tests for `createFirelineClient`. Mock fetch: `POST /v1/sandboxes` → returns `SandboxDescriptor`, `GET /v1/sandboxes/{id}` → returns status, `DELETE /v1/sandboxes/{id}` → returns 200. Exercises `create` → `get` → `list` → `findOrCreate` → `healthCheck` → `close`.

2. **`test/sandbox.test.ts`** — unit tests for the `Sandbox` handle object. Mock fetch: `sandbox.status()`, `sandbox.execute()`, `sandbox.destroy()`. Exercises `newSession()` with a mock WebSocket (verify the ACP `initialize` + `newSession` handshake fires correctly).

3. **`test/client-integration.test.ts`** — integration test replacing `host.test.ts`. Spawns the real control plane + fireline binary. Full lifecycle: `client.create()` → `sandbox.status()` → `sandbox.execute('echo hello')` → `sandbox.newSession()` → `session.prompt('hi')` → `sandbox.destroy()`. This is the "golden path" test that proves the whole stack works end-to-end.

### What the test suite proves after M4

- `client.test.ts` → `FirelineClient` factory works against a mock server (unit)
- `sandbox.test.ts` → `Sandbox` handle delegates correctly (unit)
- `client-integration.test.ts` → full lifecycle against real binaries (integration)
- `sandbox-local.test.ts` → Node subprocess sandbox works (unit)
- `catalog.test.ts` → agent catalog resolves (unit)
- `topology.test.ts` → combinator composition (unit)
- `acp.test.ts` → ACP connectivity (unit)
- `tier5-smoke.browser.test.ts` → browser harness UI (e2e via Playwright)

Every test except the integration test and the browser e2e can run without compiling Rust binaries — they're pure TS unit tests with mock fetch. This is a material improvement: today `test/host.test.ts` requires `cargo build` in its `beforeAll`, making the TS test suite slow and cargo-dependent. Under the new model, the unit tests are fast and independent; only the integration test needs binaries.

---

## Appendix: type mapping — old → new

| Old type | New type | Notes |
|---|---|---|
| `Host` interface | `FirelineClient` | `provision` → `create`, `wake` → deleted (status-polling), `status` → `sandbox.status()`, `stop` → `sandbox.destroy()` |
| `HostHandle` | `Sandbox` handle | Carries the same `id`, `acp`, `state` fields but is now a live object with methods |
| `HostStatus` | `SandboxStatus` | Same variants, different names: `created` → `creating`, `running` → `ready`, `needs_wake` → deleted (status-polling handles this), `error` → `broken` |
| `WakeOutcome` | deleted | The `wake` verb collapses: `noop` = sandbox is already ready; `blocked` = sandbox is stopped, call `client.create()` to re-provision; `advanced` = not used in the TS client layer |
| `ProvisionSpec` | `SandboxConfig` | Same fields with better names: `agentCommand`, `topology`, `resources`, `metadata` → `envVars` + `labels` (split from opaque metadata into typed fields) |
| `FirelineHostOptions` | `FirelineClientOptions` | `controlPlaneUrl` → `serverUrl`, `sharedStateUrl` → deleted (server-side concern) |
| `createFirelineHost()` | `createFirelineClient()` | Returns `FirelineClient`, not `Host` |
| `createHostedApiHost()` | `createFirelineClient()` | Same function, different server URL |
| `createFirelineClient()` (the old orchestrator bundle) | deleted | The orchestrator is a standalone concern |
| `Sandbox` interface (old, for tool execution) | merged into `Sandbox` handle | `execute()` replaces the old `Sandbox.execute(handle, call)` pattern |
| `SandboxHandle` (old) | deleted | The new `Sandbox` IS the handle |
| `ToolCall` / `ToolResult` | `ExecutionResult` | Structured tool calls become `sandbox.execute()` with a command string; typed tool dispatch is a higher-level wrapper |
| `SandboxSpec` | merged into `SandboxConfig` | `runtime_key` → `config.labels` for sandbox lookup |

---

## Appendix: module layout — old → new

```
packages/client/src/                    packages/client/src/
├── host.ts              DELETED        ├── client.ts           NEW (FirelineClient + createFirelineClient)
├── host/                DELETED        ├── sandbox.ts          NEW (Sandbox handle class)
├── host-fireline/       DELETED        ├── types.ts            NEW (SandboxConfig, SandboxStatus, ExecutionResult, etc.)
├── host-hosted-api/     DELETED        │
├── sandbox/             MERGED         │
│                                       │
├── core/                KEPT           ├── core/               KEPT
├── orchestration/       KEPT           ├── orchestration/      KEPT (WakeHandler wraps client.get + status polling)
├── sandbox-local/       KEPT           ├── sandbox-local/      KEPT
├── catalog.ts           KEPT           ├── catalog.ts          KEPT
├── acp.ts               KEPT           ├── acp.ts              KEPT (consumed by Sandbox.newSession internally)
├── acp-core.ts          KEPT           ├── acp-core.ts         KEPT
├── acp.browser.ts       KEPT           ├── acp.browser.ts      KEPT
├── browser.ts           KEPT           ├── browser.ts          KEPT
├── topology.ts          KEPT           ├── topology.ts         KEPT
└── index.ts             UPDATED        └── index.ts            UPDATED (re-exports from client.ts, types.ts, core/)
```
