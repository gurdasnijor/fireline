# RFC: Durable Promises

> Status: design rationale
> Audience: engineers deciding whether Fireline should expose a durable wait as a subscriber profile or as imperative workflow code

Fireline already has the hard part of durable waiting: the durable-subscriber substrate can write a wait point to the state stream, survive restart, and resume when the matching completion arrives later.

Durable promises exist because that substrate is host-shaped, not workflow-shaped.

`DurableSubscriber::Passive` is the right primitive when you are defining how Fireline observes events, keys completions, and rebuilds progress from the durable log. `ctx.awakeable<T>()` is the right primitive when you are writing application logic and need to say, plainly, "pause here until this resolves."

That is the core decision:

- keep one durable waiting substrate
- give it an imperative spelling for workflow authors
- refuse a second queue, id scheme, or replay engine for awakeables

## The Problem Fireline Is Solving

The workflow cases that matter most are usually easy to describe in one sentence:

- wait for a human approval
- send a request to another system and wait for the callback
- wait for legal and security to both respond
- race a human answer against a timer

Those are not exotic orchestration problems. They are ordinary product workflows with long-lived waits.

Without an imperative surface, authors are forced to think in infrastructure terms too early: define a passive subscriber, route the resolution path, then manually reconnect the resumed control flow. That is correct, but it is the wrong level of abstraction for the person writing the business logic.

Fireline needs a surface that preserves durability without making every workflow author think like a host implementer.

## The Decision

Durable promises make the passive subscriber model readable from inside workflow code.

In practice, that means:

- `ctx.awakeable<T>()` declares a durable wait point
- awaiting the returned promise suspends on the same completion key a passive subscriber would use
- resolving it appends the same completion envelope the subscriber substrate already understands
- replay rebuilds the same unresolved wait from the stream after restart

The promise is imperative syntax over a durable subscriber fact.

This is why the design frames awakeables as imperative sugar over `DurableSubscriber::Passive`, not as a parallel primitive. Fireline already knows how to wait durably. The promise surface only changes how that wait is expressed to the workflow author.

## Why Fireline Refuses A Second Workflow Engine

The failure mode here is obvious: an "easy" imperative API often grows its own resolver, its own ids, its own replay state, and eventually its own semantics. Fireline explicitly does not want that.

If awakeables became a separate engine, Fireline would have two answers to the same questions:

- what counts as the durable identity of a wait
- where suspended state lives across crashes
- how an external actor resolves pending work
- how retries and observability line up with the original event

That split would make approvals, chat integrations, webhooks, and callback handling harder to reason about, not easier.

So Fireline keeps one answer:

- durable identity comes from the same completion-key contract as subscribers, using canonical ACP identifiers instead of an awakeable-only token
- suspended state is rebuilt from the durable stream
- external resolution is still "append the matching completion"
- the same trace context and observability story follows the work across the hop

The awakeable API is allowed to improve readability. It is not allowed to fork the architecture.

## What The Imperative Surface Buys You

The point of durable promises is not novelty. The point is that some workflows are naturally linear.

This:

```ts
const approval = ctx.awakeable<boolean>({ kind: 'prompt', sessionId, requestId })
await sendApprovalCard(approval.key)

if (!(await approval.promise)) {
  throw new Error('denied')
}
```

matches how people already think about the workflow:

1. declare the thing you are waiting for
2. expose the key to the outside world
3. continue when the answer arrives

That shape matters because it composes cleanly with normal async control flow:

- `Promise.all(...)` for multi-reviewer waits
- `Promise.race(...)` for timeout patterns
- ordinary `try` / `catch` around long-lived external callbacks

The durable substrate still does the heavy lifting. The workflow author gets a control-flow shape that reads top to bottom.

## Why This Fits Fireline Specifically

Fireline already separates concerns cleanly:

- durable subscribers are the host-facing substrate for observing, delivering, retrying, and tracking durable work
- workflow code is where authors express domain logic

Durable promises sit exactly on that seam.

They let workflow code stay in the agent plane while the infrastructure plane continues to own cursor progress, retry policy, and dead-letter handling. The promise surface does not leak subscriber internals into user code, and it does not ask user code to manage transport concerns that belong to the host.

That boundary is what makes the abstraction trustworthy. The user gets a simple mental model without silently taking ownership of infrastructure details.

## Resolution Should Be Source-Agnostic

One of the strongest design choices in Fireline is that the waiter does not care who resolves it.

An awakeable may be completed by:

- a dashboard
- a Telegram bot
- a webhook receiver
- another durable-subscriber profile
- an approval-specific convenience API

That source-agnostic behavior is only possible because all of those resolution paths converge on the same durable completion contract.

From the workflow's point of view, there is just a promise that eventually resolves.

From the architecture's point of view, there is just a completion appended against the canonical key.

That is the right split.

## What Durable Promises Do Not Try To Be

Durable promises do not turn Fireline into a general deterministic workflow VM.

Fireline is not trying to replay arbitrary user code instruction by instruction between wait points. The guarantee is narrower and more useful:

- the wait point is durable
- the completion is durable
- restart rebuilds the wait from the stream
- code between waits is still ordinary application code

That tradeoff keeps the model small enough to understand and strong enough to trust.

It also keeps Fireline aligned with its broader architecture: durable streams are the source of truth, while higher-level surfaces stay thin and composable.

## When To Reach For Durable Promises

Use durable promises when the workflow author owns the question "what should happen next once this thing resolves?"

Use durable subscribers when the system design question is "how should Fireline observe, deliver, or wait on this class of event durably?"

Many real workflows use both:

- a durable subscriber profile exposes an event to the outside world
- an awakeable gives the workflow a clean imperative place to wait for the answer

That pairing is not accidental. It is the point of the design.

## References

- [Durable Subscribers](../durable-subscriber.md)
- [Approvals](../approvals.md)
- [Proposal: Durable Promises](../../proposals/durable-promises.md)
- [Proposal: Durable Subscriber Primitive](../../proposals/durable-subscriber.md)
