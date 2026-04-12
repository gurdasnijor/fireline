# Concepts

## 1. A harness spec is pure data

The TypeScript client models an agent definition as serializable data:

- `sandbox(...)` builds a `SandboxDefinition`
- `middleware([...])` builds a `MiddlewareChain`
- `agent([...])` builds an `AgentConfig`
- `compose(...)` combines them into a `Harness`

See:

- [packages/client/src/sandbox.ts](../../packages/client/src/sandbox.ts)
- [packages/client/src/types.ts](../../packages/client/src/types.ts)

The important point is that the middleware values are data, not closures. A harness can be serialized, sent to a remote host, and interpreted server-side by Rust.

```ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, trace } from '@fireline/client/middleware'

const reviewer = compose(
  sandbox({ provider: 'docker', labels: { role: 'reviewer' } }),
  middleware([trace(), approve({ scope: 'tool_calls' })]),
  agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
)
```

## 2. `start()` provisions; it does not yet return a live agent object

The proposal vocabulary talks about `start()` returning a live `FirelineAgent` object with methods like `connect()`, `resolvePermission()`, `stop()`, and `destroy()`.

That is not what the current TypeScript client implements.

Today:

- `Harness.start(options)` calls `new Sandbox(options).provision(...)`
- `StartOptions` requires `serverUrl`
- the return value is a `HarnessHandle`, which is a data handle containing:
  - `id`
  - `provider`
  - `acp`
  - `state`
  - `name`

See:

- [packages/client/src/sandbox.ts](../../packages/client/src/sandbox.ts)
- [packages/client/src/types.ts](../../packages/client/src/types.ts)

So the current split is:

- control/lifecycle handle: `HarnessHandle`
- session connection: ACP client such as `@agentclientprotocol/sdk` or `use-acp`
- observation: `createFirelineDB(...)` from `@fireline/state`
- destructive lifecycle: `SandboxAdmin` from `@fireline/client/admin`

## 3. The proposal direction vs the current implementation

The design direction in [docs/proposals/client-api-redesign.md](../proposals/client-api-redesign.md) is:

- local `start()` by default
- no URL required for local runs
- `start({ remote: '...' })` for remote
- a higher-level live object wrapping ACP + state + lifecycle

The current implementation is simpler and more explicit:

- `start({ serverUrl: 'http://127.0.0.1:4440' })`
- ACP is still a third-party concern
- state is still `@fireline/state`
- admin lifecycle is still `SandboxAdmin`

Documenting that gap matters because developers should build against the code that exists, not the proposal shorthand.

## 4. The three planes

### Control plane

Control is how you define and provision agents:

- `compose(...)`
- `Harness.start(...)`
- `Sandbox`
- `SandboxAdmin`

This is the layer that talks to the Fireline host HTTP API.

### Session plane

Session is the live conversation protocol between the client and the agent:

- ACP over `handle.acp.url`
- browser: `useAcpClient` from `use-acp`
- Node: `@agentclientprotocol/sdk`

Fireline does not wrap ACP at the package root today.

### Observation plane

Observation is the durable state stream projected into queryable collections:

- `createFirelineDB({ stateStreamUrl: handle.state.url })`
- `db.collections.sessions`
- `db.collections.promptTurns`
- `db.collections.permissions`
- `db.collections.chunks`

This is implemented in [packages/state/src/collection.ts](../../packages/state/src/collection.ts) on top of `@durable-streams/state`.

## 5. Durable streams are the source of truth

Fireline treats the durable stream as the authoritative log.

Not the process.
Not the sandbox.
Not the host memory.

You can see that assumption all through the codebase:

- approvals rebuild pending state from the stream in [crates/fireline-harness/src/approval.rs](../../crates/fireline-harness/src/approval.rs)
- the session DB is materialized from the stream in [packages/state/src/collection.ts](../../packages/state/src/collection.ts)
- provider read models fall back to stream-backed projections such as `HostIndex`

This is why observation is reactive and replayable instead of being a pile of bespoke REST polling endpoints.

## 6. Middleware is server-interpreted data

The middleware helpers in [packages/client/src/middleware.ts](../../packages/client/src/middleware.ts) only build JSON-like specs.

The translation path is:

1. TypeScript creates middleware data.
2. [packages/client/src/sandbox.ts](../../packages/client/src/sandbox.ts) maps it into topology components such as `audit`, `approval_gate`, `budget`, `context_injection`, and `peer_mcp`.
3. [crates/fireline-harness/src/host_topology.rs](../../crates/fireline-harness/src/host_topology.rs) interprets those component names and constructs the actual Rust conductor components.

That means middleware is portable and serializable by design. The behavior lives in Rust, not in user-provided JS callbacks.
