# Approval Broker

> A proxy-chain demo: the agent hits an approval gate, Fireline emits a durable permission event, an external broker forwards it to a webhook, and the run resumes from an `approval_resolved` stream append.

## Why this is uniquely Fireline

This is not an ad-hoc callback hanging off one SDK client. The approval gate is a composable ACP proxy component, and the approval decision is durable state.

- the middleware chain suspends the prompt before it reaches the agent
- the approval request is visible as a stream event
- an external system can approve later from another process
- replay and audit work because the decision is data, not an in-memory callback

That combination is what the ACP proxy-chains RFD calls the universal extension mechanism: policy lives in the proxy chain, coordination lives in the durable stream.

## What this example shows

```ts
compose(
  sandbox({}),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
  ]),
  agent(['fireline-testy-prompt']),
).start({ serverUrl: 'http://127.0.0.1:4440' })
```

Then:

1. the prompt is suspended
2. a `permission` row appears in `@fireline/state`
3. the broker observes that row and POSTs it to an external webhook
4. the webhook appends `approval_resolved`
5. the same prompt continues without reopening the session

## Run it

```bash
cargo build -q -p fireline --bin fireline --bin fireline-testy-prompt
pnpm --dir .. install --ignore-workspace --lockfile=false
cd examples/approval-broker
pnpm install
pnpm start
```

The output shows the resolved permission row and the completed prompt turn. The important point is where the decision lived: **in the stream, not in a callback held by the original client process**.
