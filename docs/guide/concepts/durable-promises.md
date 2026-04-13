# Durable Promises

Durable promises are Fireline's promise-shaped view of durable waiting.

The user-facing name is **awakeable**: a value you can `await` without giving up restart safety, replay, or canonical completion identity.

The important boundary is simple:

- awakeables are imperative sugar over `DurableSubscriber::Passive`
- they are not a second workflow engine
- the durable stream is still the source of truth

If durable subscribers are the substrate, durable promises are the workflow author's ergonomic handle on that substrate.

## The Core Idea

Without awakeables, the passive durable-subscriber story sounds like infrastructure:

1. write a durable wait event
2. derive a canonical completion key
3. wait for a matching completion envelope
4. rebuild the wait by replaying the stream after restart

With awakeables, the same flow reads like application code:

```ts
const approval = ctx.awakeable<boolean>({
  kind: 'prompt',
  sessionId,
  requestId,
})

await sendApprovalCard({ resumeKey: approval.key })

if (!(await approval.promise)) {
  throw new Error('denied')
}
```

From another process or surface, the matching completion is still just an append:

```ts
await resolveAwakeable(approval.key, true)
```

Nothing about the substrate changed. Fireline still:

- keys the wait with canonical ACP identifiers
- appends the completion to the same durable stream
- reconstructs unresolved waits by replaying that stream

The difference is where the complexity lives. Durable-subscriber plumbing stays in the runtime; application code gets to write normal async control flow.

## What An Awakeable Really Is

An awakeable is a durable wait bound to a `CompletionKey`.

That key is the whole contract:

- prompt-scoped wait: the completion is keyed by `(sessionId, requestId)`
- tool-scoped wait: the completion is keyed by `(sessionId, toolCallId)`
- on today's Rust substrate, session-scoped waits also exist for session-level wakeups keyed by `sessionId`

On `main` today, the Rust substrate makes this explicit:

- `AwakeableKey` is an alias for the durable-subscriber `CompletionKey`
- `WorkflowContext::awakeable<T>(key)` creates the wait handle
- `AwakeableResolver::resolve_awakeable(...)` appends the completion
- `AwakeableSubscriber` is the passive subscriber profile underneath

That is why durable promises do not need a new id type, a new storage table, or a second replay engine. They reuse the same completion-key spine the subscriber substrate already owns.

## What Changes And What Does Not

What changes for the workflow author:

- you can keep the pause/resume point inline with the rest of your async code
- `Promise.all(...)` and `Promise.race(...)` become the natural way to express joins and timeouts
- long waits stop looking like hand-built state machines

What does not change:

- replay still rebuilds outstanding waits from the durable stream
- resolution is still "append the matching completion envelope"
- first matching completion still wins for a given canonical key
- infra-plane cursor, retry, and dead-letter mechanics still belong to the subscriber substrate, not the awakeable API

In other words: the syntax becomes imperative, but the correctness model stays durable and log-backed.

## When To Reach For Awakeables

Use an awakeable when one workflow step wants to pause and then continue in the same control-flow block.

Good fits:

- human approval inside a larger workflow
- waiting for an external callback or vendor webhook
- pausing for two independent reviews, then joining the results
- racing a human response against a timer

Example:

```ts
const legal = ctx.awakeable<boolean>({ kind: 'step' })
const security = ctx.awakeable<boolean>({ kind: 'step' })

await notifyReviewers([legal.key, security.key])

const [legalOk, securityOk] = await Promise.all([
  legal.promise,
  security.promise,
])
```

That is the right mental model for "my code wants to wait here and continue later."

## When To Reach For Raw Durable Subscribers Instead

Use raw durable-subscriber profiles when the work belongs to host infrastructure rather than inline workflow code.

Good fits:

- deliver matching events to a webhook
- render approval cards into Telegram
- auto-approve safe actions
- fan matched events out to an external system that owns retries and acknowledgements

In those cases, the question is not "where should my function pause?" The question is "what should the host do every time this event appears on the stream?"

That is why Fireline keeps both surfaces:

- **durable subscriber** for host-owned event handling
- **durable promise / awakeable** for workflow-owned pause and resume

The substrate is shared. The authoring style is different.

## The Approval Example Is The Simplest Mental Bridge

Approvals are already the reference shape for passive durable waiting.

Today, the public API looks like approval middleware plus `resolvePermission(...)`. Durable promises give that same model a more general, more readable name:

- `permission_request` becomes "declare a wait"
- `approval_resolved` becomes "append the matching completion"
- the paused workflow becomes `await approval.promise`

That is why the durable-promises proposal describes awakeables as imperative sugar over `DurableSubscriber::Passive`, not a brand-new primitive.

## Status On `main`

The substrate is already real on `main`:

- Rust has a landed `WorkflowContext::awakeable<T>(key)` surface
- Rust has a landed `AwakeableResolver`
- replay coverage exists for "completion already present before rehydration"

The TypeScript workflow shape shown in this concept doc is the same user-facing surface that is being standardized next. That is why [docs/guide/awakeables.md](../awakeables.md) is still the runnable-status guide, while this page focuses on the durable-promises mental model.

## Read This Next

- [Awakeables](../awakeables.md)
- [Durable Subscribers](../durable-subscriber.md)
- [Approvals](../approvals.md)
- [docs/proposals/durable-promises.md](../../proposals/durable-promises.md)
