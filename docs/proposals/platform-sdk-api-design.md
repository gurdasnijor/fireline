# Platform SDK API Design

## Goal

Define the imperative TypeScript API for building applications on top of Fireline: dashboards, Slack bots, orchestrators, and custom UIs.

This proposal implements the decisions already made in [docs/gaps-platform-sdk.md](/Users/gnijor/gurdasnijor/fireline/docs/gaps-platform-sdk.md):

- `start()` returns a live `FirelineAgent`, not a raw `SandboxHandle`
- `start()` with no args boots local Fireline
- `start({ remote: '...' })` targets a remote Fireline instance
- `agent.connect()` returns the ACP SDK's `ClientSideConnection`
- `fireline.db()` is the global state entry point
- `agent.resolvePermission()` wraps `appendApprovalResolved()`
- durable approval waiting stays in Rust
- existing session-centric query builders are reused, not reinvented

## Public API

```ts
import fireline, { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, approve } from '@fireline/client/middleware'
import type { ClientSideConnection } from '@agentclientprotocol/sdk'

const db = await fireline.db()

const reviewer = await compose(
  sandbox(),
  middleware([trace(), approve({ scope: 'tool_calls' })]),
  agent(['pi-acp']),
).as('reviewer').start()

const conn = await reviewer.connect()
const { sessionId } = await conn.newSession({ cwd: '/workspace', mcpServers: [] })
await conn.prompt({ sessionId, prompt: [{ type: 'text', text: 'Review this repo.' }] })

db.permissions.subscribe((rows) => {
  const pending = rows.find((row) => row.sessionId === sessionId && row.state === 'pending')
  if (pending) {
    void reviewer.resolvePermission(sessionId, pending.requestId, { allow: true })
  }
})

await reviewer.stop()
```

## Exact TypeScript Types

### Root export

```ts
export interface FirelinePlatform {
  db(options?: FirelineDbOptions): Promise<FirelineDB>
}

declare const fireline: FirelinePlatform
export default fireline
```

### `start()` options

```ts
export interface LocalStartOptions {
  readonly remote?: undefined
  readonly name?: string
  readonly stateStream?: string
  readonly startupTimeoutMs?: number
}

export interface RemoteStartOptions {
  readonly remote: string
  readonly token?: string
  readonly name?: string
  readonly stateStream?: string
  readonly startupTimeoutMs?: number
}

export type StartOptions = LocalStartOptions | RemoteStartOptions
```

Semantics:

- `start()` or `start({})` means local mode
- `start({ remote: 'http://127.0.0.1:4440' })` means remote mode
- `serverUrl` is removed from the primary path

### `FirelineAgent`

```ts
import type {
  Client,
  ClientSideConnection,
} from '@agentclientprotocol/sdk'

export interface FirelineConnectOptions {
  readonly client?: Client | (() => Client)
  readonly initialize?: Partial<Parameters<ClientSideConnection['initialize']>[0]>
}

export interface ResolvePermissionOptions {
  readonly allow: boolean
  readonly resolvedBy?: string
}

export interface FirelineAgent<Name extends string = string> {
  readonly name: Name
  readonly id: string
  readonly provider: string
  readonly acp: Endpoint
  readonly state: Endpoint

  connect(options?: FirelineConnectOptions): Promise<ClientSideConnection>
  resolvePermission(
    sessionId: string,
    requestId: string,
    outcome: ResolvePermissionOptions,
  ): Promise<void>
  stop(): Promise<SandboxDescriptor | null>
  destroy(): Promise<SandboxDescriptor | null>
}
```

Notes:

- `connect()` returns an initialized ACP SDK `ClientSideConnection`
- the only Fireline behavior inside `connect()` is transport bridging plus ACP `initialize()`
- raw `acp` and `state` endpoints remain public as escape hatches

### `Harness` and topology return types

```ts
export interface Harness<Name extends string = string> extends HarnessSpec<Name> {
  as<NextName extends string>(name: NextName): Harness<NextName>
  start(options?: StartOptions): Promise<FirelineAgent<Name>>
}

export interface NamedTopology<Names extends string> {
  start(options?: StartOptions): Promise<Record<Names, FirelineAgent<Names>>>
}

export interface FanoutTopology<Name extends string> {
  start(options?: StartOptions): Promise<Array<FirelineAgent<Name>>>
}
```

### `fireline.db()`

```ts
import type { Collection } from '@tanstack/db'
import type {
  ChildSessionEdgeRow,
  ChunkRow,
  ConnectionRow,
  PendingRequestRow,
  PermissionRow,
  PromptTurnRow,
  RuntimeInstanceRow,
  SessionRow,
  TerminalRow,
} from './types'

export interface ObservableCollection<T extends object> extends Collection<T> {
  subscribe(callback: (rows: T[]) => void): { unsubscribe(): void }
}

export interface FirelineViews {
  queuedTurns(): ObservableCollection<PromptTurnRow>
  activeTurns(): ObservableCollection<PromptTurnRow>
  pendingPermissions(): ObservableCollection<PermissionRow>
  sessionTurns(sessionId: string): ObservableCollection<PromptTurnRow>
  connectionTurns(logicalConnectionId: string): ObservableCollection<PromptTurnRow>
  turnChunks(promptTurnId: string): ObservableCollection<ChunkRow>
  sessionPermissions(sessionId: string): ObservableCollection<PermissionRow>
}

export interface FirelineDB {
  readonly connections: ObservableCollection<ConnectionRow>
  readonly promptTurns: ObservableCollection<PromptTurnRow>
  readonly pendingRequests: ObservableCollection<PendingRequestRow>
  readonly permissions: ObservableCollection<PermissionRow>
  readonly terminals: ObservableCollection<TerminalRow>
  readonly runtimeInstances: ObservableCollection<RuntimeInstanceRow>
  readonly sessions: ObservableCollection<SessionRow>
  readonly childSessionEdges: ObservableCollection<ChildSessionEdgeRow>
  readonly chunks: ObservableCollection<ChunkRow>

  readonly views: FirelineViews

  // Compatibility alias for one release cycle.
  readonly collections: {
    readonly connections: ObservableCollection<ConnectionRow>
    readonly promptTurns: ObservableCollection<PromptTurnRow>
    readonly pendingRequests: ObservableCollection<PendingRequestRow>
    readonly permissions: ObservableCollection<PermissionRow>
    readonly terminals: ObservableCollection<TerminalRow>
    readonly runtimeInstances: ObservableCollection<RuntimeInstanceRow>
    readonly sessions: ObservableCollection<SessionRow>
    readonly childSessionEdges: ObservableCollection<ChildSessionEdgeRow>
    readonly chunks: ObservableCollection<ChunkRow>
  }

  preload(): Promise<void>
  close(): void
}

export interface FirelineDbOptions {
  readonly headers?: Readonly<Record<string, string>>
  readonly signal?: AbortSignal
}
```

Notes:

- `fireline.db()` is global, not agent-scoped
- `db.sessions` and `db.promptTurns` are top-level aliases, not `db.collections.*` only
- the seven existing query builders become `db.views.*`
- `@fireline/state` stays the implementation underneath, but is not the public import path

## `fireline.db()` Resolution Rules

`fireline.db()` must work before any agent is started:

1. If `process.env.FIRELINE_STREAM_URL` is set, use that URL.
2. Otherwise, ensure the local Fireline runtime singleton is booted.
3. Use that singleton's embedded durable-streams URL.
4. Create the DB, call `preload()`, cache it by resolved stream URL, and return it.

That gives the intended behavior:

- local scripts: `await fireline.db()` just works
- remote deployments: set `FIRELINE_STREAM_URL`, code stays unchanged

## How `start()` Changes

### Current

```ts
const handle = await compose(...).start({ serverUrl: 'http://127.0.0.1:4440' })
// handle is raw data: id, provider, acp.url, state.url
```

### Proposed

```ts
const agent = await compose(...).start()
const remoteAgent = await compose(...).start({ remote: 'http://127.0.0.1:4440' })
```

Implementation sketch inside `packages/client/src/sandbox.ts`:

1. Resolve the runtime target:
   - local: `ensureLocalRuntime()`
   - remote: `createRemoteRuntime(remote, token)`
2. Use the existing low-level provision path to obtain `SandboxHandle`
3. Materialize that handle into `FirelineAgent`

```ts
function materializeAgent<Name extends string>(
  name: Name,
  handle: SandboxHandle,
  runtime: FirelineRuntime,
): FirelineAgent<Name> {
  return {
    name,
    id: handle.id,
    provider: handle.provider,
    acp: handle.acp,
    state: handle.state,
    connect: (options) => connectAcp(handle.acp, options),
    resolvePermission: (sessionId, requestId, outcome) =>
      appendApprovalResolved({
        streamUrl: handle.state.url,
        sessionId,
        requestId,
        allow: outcome.allow,
        resolvedBy: outcome.resolvedBy,
      }),
    stop: () => runtime.admin.stop(handle.id),
    destroy: () => runtime.admin.destroy(handle.id),
  }
}
```

`Sandbox.provision()` may remain as the low-level escape hatch that still returns `SandboxHandle`. The high-level path is `start()`.

## ACP Bridge Design

`agent.connect()` absorbs `examples/shared/acp-node.ts`.

Implementation sketch:

- new internal module: `packages/client/src/acp.ts`
- input: `Endpoint` plus optional `FirelineConnectOptions`
- behavior:
  - open WebSocket
  - adapt to ACP `Stream`
  - create `ClientSideConnection`
  - initialize with caller-provided overrides merged onto sensible defaults
  - return the initialized `ClientSideConnection`

Default client handler behavior:

- `requestPermission`: returns `{ outcome: { outcome: 'cancelled' } }`
- `sessionUpdate`: no-op
- filesystem / terminal / ext methods: throw by default

That matches the current example helper, but moves it into the SDK.

## Unifying `@fireline/state` into `@fireline/client`

Publicly:

- users import `fireline` and related types from `@fireline/client`
- users do not import `createFirelineDB` from `@fireline/state`

Internally:

- `@fireline/client` keeps depending on `@fireline/state`
- `fireline.db()` calls the existing `createFirelineDB()`
- `db.views.*` call the existing seven builders from `packages/state/src/collections/`

Compatibility plan:

- keep `@fireline/state` package available for one cycle
- remove it from docs and examples immediately
- mark it as implementation detail / advanced escape hatch

## `examples/shared/` Deletions

Delete these once the new API lands:

1. `examples/shared/acp-node.ts`
   - replaced by `agent.connect()`
2. `examples/shared/resolve-approval.ts`
   - replaced by `agent.resolvePermission()`
3. `examples/shared/wait.ts`
   - unnecessary for approval handling because the conductor already waits durably in Rust
   - general state observation should use `collection.subscribe(...)`

## Migration Path

### Provision and connect

Before:

```ts
const handle = await compose(...).start({ serverUrl })
const acp = await openNodeAcpConnection(handle.acp.url, 'my-app')
const db = createFirelineDB({ stateStreamUrl: handle.state.url })
await db.preload()
```

After:

```ts
const agent = await compose(...).start({ remote: serverUrl })
const conn = await agent.connect()
const db = await fireline.db()
```

### Local mode

Before:

```ts
await compose(...).start({ serverUrl: 'http://127.0.0.1:4440' })
```

After:

```ts
await compose(...).start()
```

### Approval resolution

Before:

```ts
await appendApprovalResolved({ streamUrl: handle.state.url, sessionId, requestId, allow: true })
```

After:

```ts
await agent.resolvePermission(sessionId, requestId, { allow: true })
```

### State access

Before:

```ts
db.collections.permissions
createSessionTurnsCollection({ promptTurns: db.collections.promptTurns, sessionId })
```

After:

```ts
db.permissions
db.views.sessionTurns(sessionId)
```

### Transition compatibility

For one release:

- keep `db.collections.*` as an alias
- keep `SandboxHandle` and `Sandbox.provision()`
- keep `SandboxAdmin`
- re-export `appendApprovalResolved` from `@fireline/client/events` and optionally root for migration ease

## File-by-File Implementation Plan

### `packages/client/src/types.ts`

- add `LocalStartOptions`, `RemoteStartOptions`, `StartOptions`
- add `FirelineAgent`
- add `FirelineConnectOptions`
- add `ResolvePermissionOptions`
- add `ObservableCollection`, `FirelineViews`, `FirelineDB`, `FirelineDbOptions`
- change `Harness.start()` and topology return types from `HarnessHandle` to `FirelineAgent`
- keep `SandboxHandle` for low-level provisioning

### `packages/client/src/sandbox.ts`

- make `start(options?: StartOptions)` optional
- split local vs remote runtime resolution
- keep existing provision request builder
- materialize `SandboxHandle` into `FirelineAgent`

### `packages/client/src/topology.ts`

- update `peer`, `fanout`, `pipe` to return `FirelineAgent` values
- preserve shared `stateStream` semantics

### `packages/client/src/admin.ts`

- add `stop(id: string): Promise<SandboxDescriptor | null>`
- keep `get`, `list`, `destroy`, `status`, `healthCheck`

### `packages/client/src/events.ts`

- keep `appendApprovalResolved()`
- use it from `FirelineAgent.resolvePermission()`

### `packages/client/src/index.ts`

- export default `fireline`
- keep `compose`, `agent`, `sandbox`, `middleware`
- re-export the new `FirelineAgent`, `FirelineDB`, and related types

### `packages/client/src/platform.ts` (new)

- own the shared runtime/bootstrap singleton
- implement `fireline.db()`
- resolve local vs env-configured stream URL
- memoize DB instances by stream URL

### `packages/client/src/acp.ts` (new)

- internal ACP transport bridge
- absorbs the Node helper logic from `examples/shared/acp-node.ts`

### `packages/client/package.json`

- keep `.` / `./middleware` / `./admin` / `./resources`
- no public `@fireline/state` dependency in examples or docs

### `packages/state/src/collection.ts`

- likely no behavior change
- optionally expose a small internal helper if needed to decorate derived collections with `.subscribe()`

### `packages/state/src/collections/*.ts`

- no new query logic
- reused via `db.views.*`

## Scope Boundaries

This proposal does not add:

- a new approval waiting primitive in TypeScript
- a Fireline-specific ACP wrapper API
- a runtime filesystem browsing API beyond what ACP already covers for file reads
- a React-specific surface

Those can follow, but they are not required to close the platform SDK gaps identified in the current examples.

## Recommendation

Implement this in two passes:

1. `FirelineAgent`, `agent.connect()`, `agent.resolvePermission()`, `db.views.*`, and `admin.stop()`
2. shared local runtime bootstrap for `start()` and `fireline.db()`

Pass 1 collapses most of the current ACP, approval, and state-query boilerplate. Pass 2 delivers the no-arg local-first behavior promised by the final API.
