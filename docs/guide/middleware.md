# Middleware

All middleware helpers in the TypeScript client are serializable data
specs defined in
[packages/client/src/middleware.ts](../../packages/client/src/middleware.ts)
and typed in [packages/client/src/types.ts](../../packages/client/src/types.ts).

The translation path is:

1. JS builds middleware data.
2. [packages/client/src/sandbox.ts](../../packages/client/src/sandbox.ts)
   maps each middleware item to a topology component or tracer config.
3. [crates/fireline-harness/src/host_topology.rs](../../crates/fireline-harness/src/host_topology.rs)
   instantiates the Rust implementation.

## `trace()`

```ts
import { trace } from '@fireline/client/middleware'

const mw = trace({
  streamName: 'audit:demo',
  includeMethods: ['session/new', 'session/prompt'],
})
```

- topology component: `audit`
- Rust implementation: `AuditTracer`
  ([`audit.rs`](../../crates/fireline-harness/src/audit.rs))

Observes ACP/MCP trace events, serializes `AuditRecord` JSON, appends to
a durable stream.

## `approve({ scope })`

```ts
import { approve } from '@fireline/client/middleware'

const mw = approve({ scope: 'tool_calls', timeoutMs: 60_000 })
```

- topology component: `approval_gate`
- Rust implementation: `ApprovalGateComponent`
  ([`approval.rs`](../../crates/fireline-harness/src/approval.rs))

Emits a `permission_request` event to the state stream, blocks until an
`approval_resolved` event appears on the same stream. The wait is
durable via SSE, not in-memory callbacks.

Current limitation: `scope: 'tool_calls'` accepts that vocabulary in the
public API, but enforcement is still at the prompt level until ACP/MCP
tool interception lands upstream. See
[packages/client/src/sandbox.ts](../../packages/client/src/sandbox.ts)
for the fallback mapping.

Upcoming design note: the approval gate is the reference case for the
target [Durable Subscriber Primitive](../proposals/durable-subscriber.md).
That proposal is ahead of the current runtime, but it documents the
general durable workflow shape this middleware is expected to collapse
into.

## `budget({ tokens })`

```ts
import { budget } from '@fireline/client/middleware'

const mw = budget({ tokens: 50_000 })
```

- topology component: `budget`
- Rust implementation: `BudgetComponent`
  ([`budget.rs`](../../crates/fireline-harness/src/budget.rs))

Counts approximate prompt tokens as `ceil(chars / 4)`, enforces
`max_tokens`, terminates the current turn when the budget is exceeded.
The Rust component also supports max tool-call count and max duration,
but the TS helper only exposes `tokens` today.

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

- topology component: `context_injection`
- Rust implementation: `ContextInjectionComponent`
  ([`context.rs`](../../crates/fireline-harness/src/context.rs))

Built-in context sources: `datetime`, `workspaceFile`, `staticText`.

## `peer({ peers? })`

```ts
import { peer } from '@fireline/client/middleware'

const mw = peer({ peers: ['agent:reviewer', 'agent:writer'] })
```

- topology component: `peer_mcp`
- Rust implementation: `PeerComponent`
  ([`host_topology.rs`](../../crates/fireline-harness/src/host_topology.rs),
   [`peer/mod.rs`](../../crates/fireline-tools/src/peer/mod.rs))

Intercepts `session/new`, injects a per-session MCP server, exposes peer
tools (`list_peers`, `prompt_peer`), emits tool descriptors for the peer
MCP surface.

The optional `peers` list is now forwarded to the topology component
config — prior versions silently dropped it, this is fixed.

## `secretsProxy({ ... })`

Credential isolation is shipped end-to-end. The agent never sees
plaintext; the harness resolves secrets at call time and writes an audit
envelope to the durable stream.

```ts
import { secretsProxy } from '@fireline/client/middleware'

const mw = secretsProxy({
  GITHUB_TOKEN:      { ref: 'secret:gh-pat', allow: 'api.github.com' },
  OPENAI_API_KEY:    { ref: 'env:OPENAI_API_KEY' },
  ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' },
})
```

- topology component: `secrets_injection`
- Rust implementation: `SecretsInjectionComponent`
  ([`secrets.rs`](../../crates/fireline-harness/src/secrets.rs))

Each binding is a `SecretBinding`:

| Field | Purpose |
|---|---|
| `ref` | Credential reference resolved by the host. Common forms: `env:VAR`, `secret:key`, `oauth:provider:account`. |
| `allow` | Optional domain allow-list (string or array). The secret is only injected on outbound requests to matching hosts. |

Resolution today:

- `env:*` reads from the host process environment
- `secret:*` reads from `~/.config/fireline/secrets.toml`, falling back
  to a normalized env var name
- `oauth:provider[:account]` reads from the same TOML file, falling back
  to `FIRELINE_OAUTH_<PROVIDER>_<ACCOUNT>`

Resolved plaintext stays wrapped in the Rust-side `SecretValue` (zeroized
on drop, never serialized). The agent-visible `ToolDescriptor` surface
never contains credential parameters.

End-to-end example:

```ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, secretsProxy, trace } from '@fireline/client/middleware'

const reviewer = await compose(
  sandbox({ provider: 'docker', image: 'node:22-slim' }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    secretsProxy({
      GITHUB_TOKEN: { ref: 'secret:gh-pat', allow: 'api.github.com' },
    }),
  ]),
  agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
).start({ serverUrl: 'http://127.0.0.1:4440' })
```

Current scope of the injection target:

- **shipped:** session-scoped environment-variable injection (the
  `EnvVar` target), which covers most agent credentials today
- **Rust has, not yet exposed from TS:** `McpServerHeader` and `ToolArg`
  targets. The `SecretBinding` type accepts only the env-var shape right
  now; broader targets are tracked in the secrets injection proposal.

See also:
[docs/proposals/secrets-injection-component.md](../proposals/secrets-injection-component.md).

Upcoming design note: credential resolution is currently a dedicated
harness component, but the broader durable workflow direction is tracked
in [Durable Subscriber Primitive](../proposals/durable-subscriber.md).
Treat that as target design, not the current middleware contract.
