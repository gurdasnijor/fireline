# Durable Subscribers

A durable subscriber is Fireline's way to turn stream events into durable workflows instead of one-off adapters.

Use this substrate when you want Fireline to do one of two things reliably:

- wait for a completion that may arrive later
- push a matching event out to another system and track that delivery durably

The important idea is that both flows use the same state-stream spine. Fireline does not need a custom bridge for approvals, webhooks, Telegram, or peer delivery. It needs one durable subscriber model with different profiles.

## What This Does

Reach for a durable subscriber when your workflow depends on an event that should survive restarts, retries, and host boundaries. Common examples:

- an approval gate that waits for a human decision
- a webhook that sends a permission request to Slack or another service
- a Telegram card that lets an operator approve from chat
- an auto-approver for safe environments

Why this is better than a one-off integration:

- completion identity comes from the agent event itself
- progress is rebuilt from the durable log after restart
- retries and dead-letter handling live in one place
- trace context stays attached across the hop

## Passive Vs Active

There are two modes to remember:

| Mode | What Fireline does | Good fit | Shipped example |
| --- | --- | --- | --- |
| Passive | writes a durable request, then waits for someone else to complete it | human approval, external admin action, long-lived review loop | `approve(...)` |
| Active | matches an event and performs the side effect itself | webhooks, Telegram delivery, auto-approve, internal routing | `webhook(...)`, `telegram(...)`, `autoApprove()` |

The simplest user-facing rule:

- use passive when another system will make the decision
- use active when Fireline should deliver or act automatically

## Fastest Way To See The Model

The easiest replay on `main` is still the approval capture, because it shows the passive half of the model clearly:

```bash
export FIRELINE_BIN="$PWD/target/debug/fireline"
export FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams"
export FQA_APPROVAL_AGENT_COMMAND="$PWD/target/debug/fireline-testy-fs"

node docs/demos/scripts/replay-fqa-approval.mjs
```

Expected output excerpt:

```json
{
  "summaryPath": "/.../.tmp/fqa-approval-demo/latest/summary.json",
  "allowVerdict": "pass",
  "denyVerdict": "pass",
  "promptLevelFallback": true
}
```

What that proves:

- a `permission_request` is written durably
- an external actor can resolve it later
- the original run resumes from the same durable workflow

Then, when you want Fireline to deliver that same event outward for you, you swap in an active profile such as `webhook(...)` or `telegram(...)`.

## The User-Facing Profiles On `main`

These are the durable-subscriber profiles you can reach from the TypeScript middleware surface today:

- `approve({ scope: 'tool_calls' })`
  Passive approval gate. It emits a durable request and waits for a matching resolution.
- `webhook({ url, events, keyBy, retry })`
  Active profile. It posts matching events to an HTTP endpoint.
- `telegram({ token, events, keyBy, retry })`
  Active profile. It renders matching events into Telegram.
- `autoApprove()`
  Active profile. It resolves matching approvals automatically.

The advanced helper is also exported:

```ts
import { durableSubscriber } from '@fireline/client/middleware'
```

Most users should prefer the named helpers above. They carry the right defaults and read better in a spec.

## Completion Keys: What `keyBy` Means

Durable subscribers do not ask you to invent a random string key. They key work using canonical identifiers that already exist in the agent event.

The user-facing `keyBy` values are:

| `keyBy` | Use it when | Why |
| --- | --- | --- |
| `session_request` | one event should resolve once per prompt/request | good default for approvals and approval notifications |
| `session_tool_call` | one event should resolve once per tool call | good fit for delivery or routing tied to a specific tool call |

Why this matters:

- retries dedupe against the same logical work item
- downstream systems can stay idempotent
- you do not need to invent your own correlation ids

Example:

```ts
import { webhook } from '@fireline/client/middleware'

const approvalsOut = webhook({
  url: 'https://example.com/fireline/approvals',
  events: ['permission_request'],
  keyBy: 'session_request',
  retry: { maxAttempts: 3, initialBackoffMs: 1_000 },
})
```

This says:

- send every `permission_request` to this endpoint
- treat `(sessionId, requestId)` as the durable completion identity
- retry a few times before dead-lettering

## At-Least-Once Delivery

Active durable subscribers are at-least-once, not exactly-once.

That is deliberate. If Fireline sends a webhook and crashes before it records forward progress, it must be allowed to send that event again after restart.

What you should assume downstream:

- your receiver may see the same logical event more than once
- your receiver should dedupe by the canonical key, not by arrival count
- retries are normal behavior, not a Fireline bug

In practice:

- use `session_request` for approval-shaped flows
- use `session_tool_call` for tool-call-shaped flows
- make the receiver idempotent

## Cursor Monotonicity

Each active profile advances through the stream with a durable cursor. Fireline only moves that cursor forward after handling the matched event.

Why that matters:

- restart does not lose position
- failed deliveries can be retried safely
- later events do not silently jump ahead of earlier unfinished work

You do not manage cursor offsets yourself. The substrate owns them.

## Trace Context Travels With The Event

Durable subscribers preserve W3C trace context so the side effect stays in the same distributed trace.

The fields to know are:

- `_meta.traceparent`
- `_meta.tracestate`
- `baggage`

On current `main`, that trace context is propagated through durable-subscriber side effects and peer routing. That means:

- a webhook can join the same trace
- a Telegram action can be correlated back to the originating prompt
- cross-agent hops can stay on one distributed trace tree

## Compose Shape

This is the smallest active + passive example to remember:

```ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, trace, webhook } from '@fireline/client/middleware'

const handle = await compose(
  sandbox(),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    webhook({
      url: 'https://example.com/fireline/approvals',
      events: ['permission_request'],
      keyBy: 'session_request',
      retry: { maxAttempts: 3, initialBackoffMs: 1_000 },
    }),
  ]),
  agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
).start({ serverUrl: 'http://127.0.0.1:4440', name: 'subscriber-demo' })
```

What happens here:

- `approve(...)` creates the passive wait point
- `webhook(...)` actively sends the matching event out
- the external system decides what to do next
- the approval is still completed against the same durable key

## What Could Go Wrong

- Do not treat delivery as exactly-once.
  Your receiver must be idempotent.
- Do not invent your own completion id.
  Use the canonical key strategy the middleware already gives you.
- `approve({ scope: 'tool_calls' })` is still prompt-level fallback today.
  The durable substrate is real; the current approval match semantics are still narrower than the public wording suggests.
- `webhook(...)` currently requires a concrete `url`.
  Target-only routing is not the live path yet.

## Which Guide To Read Next

- [Approvals](./approvals.md)
  Start here if your workflow waits for a human or external decision.
- [Telegram](./telegram.md)
  Read this if you want the approval or chat surface to live in Telegram.
- [Multi-agent](./multi-agent.md)
  Read this if your durable side effect is another Fireline agent rather than a human-facing system.

## Deeper References

- [docs/demos/fqa-approval-demo-capture.md](../demos/fqa-approval-demo-capture.md)
- [docs/proposals/durable-subscriber.md](../proposals/durable-subscriber.md)
- [packages/client/src/middleware/webhook.ts](../../packages/client/src/middleware/webhook.ts)
- [packages/client/src/middleware/telegram.ts](../../packages/client/src/middleware/telegram.ts)
