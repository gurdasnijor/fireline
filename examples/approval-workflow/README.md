# Approval Workflow

The product question is simple: how do you let an agent move fast without giving it silent permission to do dangerous things? "Trust the model" is not a workflow. It is an outage postmortem waiting to happen.

This demo shows the Fireline answer. The agent reaches for a risky operation, Fireline pauses the run, emits a durable approval record, and an external workflow gets a chance to decide. That workflow can be a webhook, Slack action, email link, or any other human system. When someone approves, the agent continues from the same turn and the decision stays in the audit trail.

## What Happens

1. `approve({ scope: 'tool_calls' })` turns dangerous tool calls into approval checkpoints.
2. A tiny webhook server receives the approval request.
3. The webhook resolves the request through `handle.resolvePermission(...)`.
4. The original prompt resumes instead of restarting.

## The Code

```ts
const handle = await compose(
  sandbox({}),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    secretsProxy({ ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' } }),
  ]),
  agent(agentCommand),
).start({ serverUrl, name: 'approval-workflow' })
```

That is the product surface. The rest of the demo is ordinary glue around the durable approval event: send a notification out, receive a decision back, call `handle.resolvePermission(...)`, continue the run.

## The Primitive Behind This Example

Conceptually, this demo is the approval-gate reference case for `DurableSubscriber::Passive`: the agent emits `permission_request`, the workflow waits on `PromptKey(SessionId, RequestId)`, and the external decision completes that wait with `approval_resolved`. The concrete webhook hop in this example is one delivery shape of the broader subscriber substrate described in [docs/proposals/durable-subscriber.md](../../docs/proposals/durable-subscriber.md), especially the webhook-oriented material in [§5.2 WebhookSubscriber](../../docs/proposals/durable-subscriber.md#52-webhooksubscriber) and its Webhook Delivery Profile.

The important point is that the README's code is the current, real Fireline API surface, not a speculative rewrite. `handle.resolvePermission(...)` is today's approval-specific resolver for the same passive completion model. The proposal in [docs/proposals/durable-promises.md](../../docs/proposals/durable-promises.md) gives this mental model a more ergonomic name: awakeables. In that framing, the paused approval is an imperative `await` over the same durable completion key, but this example deliberately stays on the shipping `@fireline/client` APIs.

## Run It

```bash
pnpm --dir .. install --ignore-workspace --lockfile=false
cd examples/approval-workflow
pnpm install
ANTHROPIC_API_KEY=... \
pnpm start
```

The output shows the request id, the approval outcome, and the same prompt turn completing after the external decision comes back.
