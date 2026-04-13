# Middleware

Fireline middleware is serializable control-plane data, not userland closures.

That is the important boundary:

- TypeScript builds middleware specs
- [`packages/client/src/sandbox.ts`](../../packages/client/src/sandbox.ts) lowers each spec into topology components or tracer config
- Rust instantiates the real behavior in the harness

The path on `main` is:

1. `middleware([...])` builds a `MiddlewareChain`
2. `compose(...)` embeds that chain in the harness spec
3. the control plane lowers each item to a named component such as `approval_gate`, `peer_mcp`, `webhook_subscriber`, or `telegram`

The exported helpers live in:

- [`packages/client/src/middleware.ts`](../../packages/client/src/middleware.ts)
- [`packages/client/src/middleware/`](../../packages/client/src/middleware/)
- [`packages/client/src/types.ts`](../../packages/client/src/types.ts)

## Quick Map

| Helper | Lowers to | Reach for it when | Important note |
| --- | --- | --- | --- |
| `trace()` | `audit` | you want ACP traffic in a durable audit stream | defaults stream name to `audit:<harness-name>` |
| `approve()` | `approval_gate` | a human or external system must approve work | `scope: 'tool_calls'` is still prompt-level fallback today |
| `budget()` | `budget` | you need a hard token cap | TS exposes tokens only; Rust budget supports more knobs |
| `secretsProxy()` | `secrets_injection` | the host should inject credentials without exposing plaintext to the agent | env-var target is the only TS-exposed target today |
| `peer()` | `peer_mcp` | one agent should discover and prompt another | exposes the peer MCP surface |
| `webhook()` | `webhook_subscriber` | Fireline should deliver matched events to HTTP | `url` is required on the live surface |
| `telegram()` | `telegram` | Telegram should be the chat and approval surface | minimum-correct for the current Rust config, not full DS parity yet |
| `autoApprove()` | `auto_approve` | matching approvals should resolve automatically | TS accepts `events` / `retry`, but current lowering is still bare |

## Classic Middleware

These are the older harness components most users reach for first.

### `trace()`

```ts
import { trace } from '@fireline/client/middleware'

const audit = trace({
  streamName: 'audit:demo',
  includeMethods: ['session/new', 'session/prompt'],
})
```

- lowers to: `audit`
- Rust implementation: `AuditTracer`
  in [`crates/fireline-harness/src/audit.rs`](../../crates/fireline-harness/src/audit.rs)

What it does:

- records ACP traffic as durable audit rows
- optionally filters to specific ACP methods
- defaults the stream name to `audit:<harness-name>` when you omit `streamName`

Use it when:

- you want a replayable activity log
- you are building an observability pane or audit trail
- you want the same trace stream locally and on hosted Fireline

### `approve({ scope, timeoutMs })`

```ts
import { approve } from '@fireline/client/middleware'

const gate = approve({
  scope: 'tool_calls',
  timeoutMs: 60_000,
})
```

- lowers to: `approval_gate`
- Rust implementation: [`crates/fireline-harness/src/approval.rs`](../../crates/fireline-harness/src/approval.rs)

What it does:

- emits a durable `permission_request`
- waits for a matching `approval_resolved`
- resumes the same session/request after the resolution appears on the stream

Use it when:

- a human needs to approve risky tool use
- an external system such as Telegram or a webhook should make the decision
- you want the approval rendezvous to survive host death and replay

Important current limitation:

- `scope: 'tool_calls'` is the right public vocabulary
- the live lowering still uses prompt-level fallback matching in
  [`packages/client/src/sandbox.ts`](../../packages/client/src/sandbox.ts)
  until upstream tool-call interception lands

Read next:

- [Approvals](./approvals.md)
- [Awakeables](./awakeables.md)
- [Durable subscribers](./durable-subscriber.md)

### `budget({ tokens })`

```ts
import { budget } from '@fireline/client/middleware'

const cap = budget({ tokens: 50_000 })
```

- lowers to: `budget`
- Rust implementation: [`crates/fireline-harness/src/budget.rs`](../../crates/fireline-harness/src/budget.rs)

What it does:

- estimates token use as roughly `ceil(chars / 4)`
- stops the current turn when the configured token cap is exceeded

Use it when:

- you need a hard ceiling for prompts or demos
- you want a fail-fast cap independent of model-side billing controls

Current gap:

- the TS helper exposes only `tokens`
- the Rust component supports broader budget knobs, but those are not surfaced from `@fireline/client` yet

### `secretsProxy({ ... })`

```ts
import { secretsProxy } from '@fireline/client/middleware'

const secrets = secretsProxy({
  GITHUB_TOKEN: { ref: 'secret:gh-pat', allow: 'api.github.com' },
  ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' },
})
```

- lowers to: `secrets_injection`
- Rust implementation: [`crates/fireline-harness/src/secrets.rs`](../../crates/fireline-harness/src/secrets.rs)

What it does:

- resolves credentials on the host side
- injects them at call time
- keeps plaintext out of the agent-visible tool surface

Use it when:

- the agent needs credentials but should never see the raw token
- you want domain-scoped outbound injection rather than broad environment exposure

What the binding means:

| Field | Meaning |
| --- | --- |
| `ref` | host-owned credential ref such as `env:VAR`, `secret:key`, or `oauth:provider[:account]` |
| `allow` | optional outbound domain allow-list |

Current TS scope:

- shipped: env-var style injection targets
- not yet surfaced from TS: broader Rust target types such as header and tool-argument injection

Read next:

- [Secrets isolation section in the root README](../../README.md)
- [docs/proposals/secrets-injection-component.md](../proposals/secrets-injection-component.md)

### `peer({ peers? })`

```ts
import { peer } from '@fireline/client/middleware'

const mesh = peer({
  peers: ['agent:reviewer', 'agent:writer'],
})
```

- lowers to: `peer_mcp`
- Rust implementation spans:
  - [`crates/fireline-tools/src/peer/mod.rs`](../../crates/fireline-tools/src/peer/mod.rs)
  - [`crates/fireline-harness/src/host_topology.rs`](../../crates/fireline-harness/src/host_topology.rs)

What it does:

- injects the peer MCP surface into the session
- makes peer discovery and peer prompting available through tools
- optionally forwards an explicit peer allow-list into the topology config

Use it when:

- one agent should ask another for help
- you want multi-agent interaction without building a bespoke router

Read next:

- [Multi-agent](./multi-agent.md)
- [Telegram](./telegram.md) if the peer replies should surface in chat

## Durable-Subscriber Profiles

These helpers are still middleware, but their runtime model is different from the classic harness components above.

They lower to active durable-subscriber profiles:

- Fireline matches events on the stream
- the subscriber performs the side effect or completion
- progress survives restart because the stream stays the source of truth

Read the substrate-level model in [Durable subscribers](./durable-subscriber.md).

### `webhook({ url, events, keyBy, headers, retry })`

```ts
import { webhook } from '@fireline/client/middleware'

const approvalsOut = webhook({
  target: 'slack-approvals',
  url: 'https://hooks.slack.com/services/demo',
  events: ['permission_request'],
  keyBy: 'session_request',
  retry: { maxAttempts: 3, initialBackoffMs: 1_000 },
})
```

- lowers to: `webhook_subscriber`
- Rust implementation: [`crates/fireline-harness/src/webhook_subscriber.rs`](../../crates/fireline-harness/src/webhook_subscriber.rs)

What it does:

- matches configured agent-plane events
- posts them to an HTTP endpoint
- advances a durable subscriber cursor only after handling the event

What the live lowering synthesizes for you:

- `target`
  defaults from `target ?? name ?? URL host`
- `cursorStream`
  becomes `subscribers:webhook:<target-slug>`
- `deadLetterStream`
  becomes `subscribers:webhook:<target-slug>:dead-letter`
- retry budget defaults to `maxAttempts = 1` when no retry policy is supplied

Current limitation:

- `url` is required on the live path
- target-only routing is still a follow-on host capability, even though `target` is accepted for naming

Use it when:

- Fireline should deliver approvals or other matched events to an external HTTP surface
- you want at-least-once durable delivery without a custom bridge process

### `telegram({ token, ... })`

```ts
import { telegram } from '@fireline/client/middleware'

const chat = telegram({
  token: { ref: 'env:TELEGRAM_BOT_TOKEN' },
  chatId: process.env.TELEGRAM_CHAT_ID,
  allowedUserIds: ['123456789'],
  scope: 'tool_calls',
})
```

- lowers to: `telegram`
- Rust implementation: [`crates/fireline-harness/src/telegram_subscriber.rs`](../../crates/fireline-harness/src/telegram_subscriber.rs)

What it does:

- turns Telegram into a chat and approval surface
- polls the Telegram Bot API
- renders approval interactions into inline cards

Live defaults:

- `scope` defaults to `'tool_calls'`
- `parseMode` defaults to `'html'`
- `pollIntervalMs` defaults to `1000`
- `pollTimeoutMs` defaults to `30000`

Important honesty note:

- the current lowering is minimum-correct for the current Rust `TelegramSubscriberConfig`
- older compatibility fields such as `events`, `keyBy`, and `retry` still exist on the TS type surface
- those compatibility fields do not currently drive the control-plane payload

Use it when:

- Telegram should be the operator-facing surface
- approvals should live in chat instead of a custom dashboard

Read next:

- [Telegram](./telegram.md)
- [Durable subscribers](./durable-subscriber.md)

### `autoApprove({ name?, events?, retry? })`

```ts
import { autoApprove } from '@fireline/client/middleware'

const safeEnv = autoApprove()
```

- lowers to: `auto_approve`
- Rust implementation: [`crates/fireline-harness/src/auto_approve.rs`](../../crates/fireline-harness/src/auto_approve.rs)

What it does:

- automatically resolves matching approval requests
- uses the same canonical approval completion path as the passive approval gate

Use it when:

- you want policy-driven approval resolution in trusted environments
- you want a demo or CI harness to progress without manual intervention

Current limitation:

- the TS helper accepts `events` and `retry`
- the current lowering still emits only the bare `auto_approve` component with no config payload
- so those options are accepted at the TS surface but are not yet effective host-side

## Advanced And Adjacent Helpers

These are also exported from `@fireline/client`, but they are not the focus of this page:

- `contextInjection(...)` and `inject([...])`
  lower to `context_injection` for prompt preloading
- `attachTools(...)`
  lowers to `attach_tool` for launch-time capability attachment
- `durableSubscriber(...)`
  thin identity helper for advanced durable-subscriber profile construction
- `peerRouting()`
  lowers to `peer_routing`
- `wakeDeployment()`
  lowers to `always_on_deployment`

If you are writing ordinary app code, prefer the named helpers (`webhook`, `telegram`, `autoApprove`) over `durableSubscriber(...)` directly.

## Composition Example

This is the shape to keep in your head:

```ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import {
  approve,
  autoApprove,
  budget,
  peer,
  secretsProxy,
  telegram,
  trace,
  webhook,
} from '@fireline/client/middleware'

const spec = compose(
  sandbox(),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    budget({ tokens: 250_000 }),
    secretsProxy({
      ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' },
    }),
    peer({ peers: ['agent:reviewer'] }),
    webhook({
      url: 'https://example.com/fireline/approvals',
      events: ['permission_request'],
      keyBy: 'session_request',
    }),
    telegram({
      token: { ref: 'env:TELEGRAM_BOT_TOKEN' },
      scope: 'tool_calls',
    }),
    autoApprove(),
  ]),
  agent(['pi-acp']),
)
```

You would not normally use every helper at once. The point is that they all share the same composition model: serializable specs lowered into one harness topology.

## Read This Next

- [Approvals](./approvals.md)
- [Awakeables](./awakeables.md)
- [Durable subscribers](./durable-subscriber.md)
- [Telegram](./telegram.md)
- [Multi-agent](./multi-agent.md)
