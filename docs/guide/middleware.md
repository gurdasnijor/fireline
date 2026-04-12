# Middleware

All middleware helpers in the TypeScript client are serializable data specs defined in [packages/client/src/middleware.ts](../../packages/client/src/middleware.ts) and typed in [packages/client/src/types.ts](../../packages/client/src/types.ts).

The translation path is:

1. JS builds middleware data.
2. [packages/client/src/sandbox.ts](../../packages/client/src/sandbox.ts) maps each middleware item to a topology component or tracer config.
3. [crates/fireline-harness/src/host_topology.rs](../../crates/fireline-harness/src/host_topology.rs) instantiates the Rust implementation.

## `trace()`

```ts
import { trace } from '@fireline/client/middleware'

const mw = trace({
  streamName: 'audit:demo',
  includeMethods: ['session/new', 'session/prompt'],
})
```

What it becomes:

- topology component name: `audit`
- Rust implementation: `AuditTracer`
- source: [crates/fireline-harness/src/audit.rs](../../crates/fireline-harness/src/audit.rs)

What it does:

- observes ACP/MCP trace events
- serializes `AuditRecord` JSON
- appends those records to a durable stream

## `approve({ scope })`

```ts
import { approve } from '@fireline/client/middleware'

const mw = approve({ scope: 'tool_calls', timeoutMs: 60_000 })
```

What it becomes:

- topology component name: `approval_gate`
- Rust implementation: `ApprovalGateComponent`
- source: [crates/fireline-harness/src/approval.rs](../../crates/fireline-harness/src/approval.rs)

What it does:

- emits a `permission_request` event to the state stream
- blocks until an `approval_resolved` event appears on that same stream
- waits durably via SSE, not in-memory callbacks

Important current limitation:

- the TS helper accepts `scope: 'tool_calls'`
- the current implementation still gates at the prompt level, not the individual tool-call level
- that fallback is visible in [packages/client/src/sandbox.ts](../../packages/client/src/sandbox.ts), where `scope: 'tool_calls'` is translated into a prompt-level approval policy until ACP/MCP tool interception lands upstream

## `budget({ tokens })`

```ts
import { budget } from '@fireline/client/middleware'

const mw = budget({ tokens: 50_000 })
```

What it becomes:

- topology component name: `budget`
- Rust implementation: `BudgetComponent`
- source: [crates/fireline-harness/src/budget.rs](../../crates/fireline-harness/src/budget.rs)

What it does today:

- counts approximate prompt tokens as `ceil(chars / 4)`
- enforces `max_tokens`
- terminates the current turn when the budget is exceeded

What is not wired from the TS client today:

- max tool-call count
- max duration

The Rust component supports those concepts, but the current TS middleware helper only exposes `tokens`.

## `contextInjection(...)` and `inject([...])`

```ts
import { contextInjection, inject } from '@fireline/client/middleware'

const a = contextInjection({
  prependText: 'Repository policy goes here.',
  placement: 'prepend',
})

const b = inject([
  { kind: 'datetime' },
  { kind: 'workspaceFile', path: '/workspace/README.md' },
  { kind: 'staticText', text: 'Use pnpm, not npm.' },
])
```

What they become:

- topology component name: `context_injection`
- Rust implementation: `ContextInjectionComponent`
- source: [crates/fireline-harness/src/context.rs](../../crates/fireline-harness/src/context.rs)

Built-in context sources that exist today:

- `datetime`
- `workspaceFile`
- `staticText`

## `peer()`

```ts
import { peer } from '@fireline/client/middleware'

const mw = peer()
```

What it becomes:

- topology component name: `peer_mcp`
- Rust implementation: `PeerComponent`
- sources:
  - [crates/fireline-harness/src/host_topology.rs](../../crates/fireline-harness/src/host_topology.rs)
  - [crates/fireline-tools/src/peer/mod.rs](../../crates/fireline-tools/src/peer/mod.rs)

What it does:

- intercepts `session/new`
- injects a per-session MCP server
- exposes peer tools such as `list_peers` and `prompt_peer`
- emits tool descriptors for the peer MCP surface

## `secretsProxy()`: not in the TS client yet

The Rust side has a real secrets-injection implementation:

- [crates/fireline-harness/src/secrets.rs](../../crates/fireline-harness/src/secrets.rs)

That code includes:

- `SecretsInjectionComponent`
- `InjectionRule`
- `InjectionTarget`
- `InjectionScope`
- credential resolvers and redacted `SecretValue`

What it does on the Rust side today:

- resolves credentials on the prompt path
- supports session-scoped environment-variable injection end to end
- keeps plaintext wrapped in `SecretValue`

What does **not** exist today:

- a `secretsProxy()` helper in `packages/client/src/middleware.ts`
- a TS serialization path from `compose(...)` into `SecretsInjectionComponent`

So if you are writing against the public TS client, treat secrets injection as planned-but-not-exposed.
