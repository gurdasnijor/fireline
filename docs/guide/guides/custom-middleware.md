# Write Custom Middleware

You have an infrastructure problem, not a prompt problem.

Maybe you need a new guardrail before prompts go out. Maybe you need approval
events delivered to your own system. Maybe you need a host-owned side effect
that survives retries and restarts. That is when you add middleware to
Fireline.

This guide is for the landed extension surface on current `main`: the
TypeScript builder in `@fireline/client`, the lowering step in
[`packages/client/src/sandbox.ts`](../../packages/client/src/sandbox.ts), and
the Rust runtime registration in
[`crates/fireline-harness/src/host_topology.rs`](../../crates/fireline-harness/src/host_topology.rs)
plus the durable-subscriber substrate in
[`crates/fireline-harness/src/durable_subscriber.rs`](../../crates/fireline-harness/src/durable_subscriber.rs).

## Start with the right question

Before you add a new `kind`, ask this first:

- Can existing middleware plus your own endpoint solve it already?
  If `webhook()` or `telegram()` can do the job, use that first.
- Does the behavior need to sit in the ACP request path, or react to stream
  events after the fact?

That answer tells you which pattern to copy.

## Pick the shape

| If your middleware looks like... | Copy this pattern | Why |
| --- | --- | --- |
| serializable config for a synchronous component | `trace()`, `budget()`, `secretsProxy()` | The TypeScript side just returns `{ kind, ...options }`, and `sandbox.ts` lowers it to one topology component |
| a durable gate that waits for some later completion | `approve()` | The runtime emits a stream event and waits for a matching completion on the durable stream |
| a host-owned side effect with retry and dead-letter behavior | `webhook()`, `telegram()`, `autoApprove()` | The TypeScript side normalizes a declarative profile; the Rust side implements an active durable subscriber |

If you are unsure, start by matching the smallest existing shape that solves
the same problem.

## Recipe 1: Add a classic component-style middleware

Use this path for middleware that is just configuration for a runtime component.
`budget()` is the cleanest example.

### 1. Build a serializable TypeScript helper

The builder should stay thin. `budget()` is exactly the shape to copy:

```ts
export function budget(options: {
  readonly tokens?: number
} = {}): BudgetMiddleware {
  return {
    kind: 'budget',
    ...cloneDefined(options),
  }
}
```

The important rules are:

- keep it declarative
- keep `kind` stable
- do not accept callbacks or closures
- strip `undefined` fields so the wire format stays clean

`trace()` and `approve()` follow the same rule, even though they drive different
runtime behavior.

### 2. Lower that `kind` in `sandbox.ts`

The next step is the switch in
[`packages/client/src/sandbox.ts`](../../packages/client/src/sandbox.ts).
That is where a middleware spec turns into a named topology component.

Current `budget` lowering:

```ts
case 'budget':
  return [
    {
      name: 'budget',
      config: {
        ...(middleware.tokens !== undefined ? { maxTokens: middleware.tokens } : {}),
      },
    },
  ]
```

This is the real contract boundary. Your custom TypeScript helper does not run
inside the sandbox. It only produces JSON that the Rust side knows how to load.

### 3. Register the runtime component in Rust

`host_topology.rs` is where the named component becomes a real Rust object.

Current `budget` registration:

```rust
.register_component("budget", move |config| {
    let parsed = config
        .cloned()
        .map(serde_json::from_value::<BudgetComponentConfig>)
        .transpose()
        .context("parse budget config")?
        .unwrap_or(BudgetComponentConfig { max_tokens: None });
    Ok(sacp::DynConnectTo::new(BudgetComponent::new(
        BudgetConfig {
            max_tokens: parsed.max_tokens,
            max_tool_calls: None,
            max_duration: None,
            on_exceeded: BudgetAction::TerminateTurn,
        },
    )))
})
```

That pattern is the contributor recipe for `trace()`, `budget()`,
`secretsProxy()`, and the current prompt-level `approve()` fallback: parse the
config, build one runtime component, and return it as a topology node.

## Recipe 2: Add a durable-subscriber style middleware

Use this path when the middleware needs host-owned work that outlives any one
prompt call: webhook delivery, Telegram approval cards, auto-approval, or a
similar background action.

### 1. Normalize the TypeScript shape

The subscriber-style helpers all flow through
[`packages/client/src/middleware/shared.ts`](../../packages/client/src/middleware/shared.ts).

`webhook()` is the clearest shape:

```ts
export function webhook(options: WebhookOptions): WebhookMiddleware {
  if (!options.url) {
    throw new Error(
      'webhook middleware currently requires url for live lowering; target-only routing is pending host target config support',
    )
  }

  return durableSubscriber({
    kind: 'webhook',
    ...options,
  })
}
```

What `durableSubscriber(...)` buys you:

- cloned `events` selectors
- cloned retry policy
- cloned secret refs for headers or tokens
- one consistent declarative profile shape across `webhook()`, `telegram()`, and
  `autoApprove()`

### 2. Lower the profile to a concrete runtime config

Subscriber-style middleware still passes through `sandbox.ts`, but the payload
is usually richer than a one-field component config.

For `webhook()`, `sandbox.ts` builds the delivery URL, cursor stream, dead-letter
stream, and retry policy. For `telegram()`, it builds the current
`TelegramSubscriberConfig` shape from token, chat routing, poll intervals, and
approval timeout.

The rule is the same as the classic path: the TypeScript side normalizes data,
the Rust side owns behavior.

### 3. Implement the subscriber contract in Rust

The landed subscriber traits are in
[`crates/fireline-harness/src/durable_subscriber.rs`](../../crates/fireline-harness/src/durable_subscriber.rs):

```rust
pub trait DurableSubscriber: Send + Sync {
    type Event: DeserializeOwned + Send + Sync + 'static;
    type Completion: Serialize + Send + Sync + 'static;

    fn name(&self) -> &str;
    fn matches(&self, envelope: &StreamEnvelope) -> Option<Self::Event>;
    fn completion_key(&self, event: &Self::Event) -> CompletionKey;
    fn is_completed(&self, event: &Self::Event, log: &[StreamEnvelope]) -> bool;
}

pub trait PassiveSubscriber: DurableSubscriber {}

#[async_trait]
pub trait ActiveSubscriber: DurableSubscriber {
    async fn handle(&self, event: Self::Event) -> HandlerOutcome<Self::Completion>;
}
```

Copy the existing patterns:

- `ApprovalGateSubscriber` is the passive shape
- `WebhookSubscriber`, `TelegramSubscriber`, and `AutoApproveSubscriber` are the
  active shapes

The important invariant is `completion_key(...)`: derive it from canonical ACP
ids that are already on the event, not from user-supplied strings. The runtime
already gives you `CompletionKey::Prompt`, `CompletionKey::Tool`, and
`CompletionKey::Session`.

### 4. Register it as active or passive

The driver keeps the two runtime modes explicit:

```rust
driver.register_passive(approval_subscriber.clone());
driver.register_active(webhook_subscriber.clone());
```

That is the line between “wait for an external completion to appear on the
stream” and “perform the side effect that eventually writes the completion.”

## A practical checklist

- Keep the TypeScript helper declarative. No callbacks, no synthetic ids, no
  hidden runtime state.
- Give the middleware one stable `kind` and one lowering path in `sandbox.ts`.
- If the runtime behavior is background or retryable, model it as a durable
  subscriber instead of a prompt-path hack.
- Derive completion identity from canonical session, request, or tool-call ids.
- Prefer host-resolved refs like `env:...` or `secret:...` over plaintext
  credentials in the TypeScript surface.
- Document any TS/Rust gap honestly. `approve({ scope: 'tool_calls' })` and
  `telegram()` both still carry live caveats on current `main`.

## Good references in tree

- [Middleware](../middleware.md) for the built-in helpers that already ship.
- [Quickstart](./quickstart.md) for the smallest working `npx fireline <spec>` path.
- [examples/telegram-demo/agent.ts](../../examples/telegram-demo/agent.ts) for an active-profile middleware array in a runnable spec.
- [crates/fireline-harness/src/auto_approve.rs](../../crates/fireline-harness/src/auto_approve.rs) for the smallest active durable-subscriber implementation.
- [crates/fireline-harness/src/webhook_subscriber.rs](../../crates/fireline-harness/src/webhook_subscriber.rs) and [crates/fireline-harness/src/telegram_subscriber.rs](../../crates/fireline-harness/src/telegram_subscriber.rs) for the heavier active-profile shapes.
