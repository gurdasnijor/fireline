# Runs And Sessions

> Related:
> - [`index.md`](./index.md)
> - [`object-model.md`](./object-model.md)
> - [`product-api-surfaces.md`](./product-api-surfaces.md)
> - [`user-surfaces.md`](./user-surfaces.md)
> - [`out-of-band-approvals.md`](./out-of-band-approvals.md)
> - [`priorities.md`](./priorities.md)
> - [`../state/session-load.md`](../state/session-load.md)
> - [`../ts/primitives.md`](../ts/primitives.md)
> - [`../execution/09-state-first-session-load.md`](../execution/09-state-first-session-load.md)
> - [`../execution/13-distributed-runtime-fabric/README.md`](../execution/13-distributed-runtime-fabric/README.md)

## Purpose

Fireline already has meaningful session foundations.

What is still missing is a clearer product contract for:

- what a **run** is
- what a **session** is
- how they relate
- which one users should think they are starting, resuming, or inspecting

This doc makes that boundary explicit.

## Short Version

- a **run** is the managed execution object
- a **session** is the durable record of what happened

Users often start a run.

Later, they inspect or resume a session.

Those two things are related, but they are not the same object.

## Why The Distinction Matters

Without a clean distinction, Fireline risks collapsing several concerns into one
ambiguous object:

- lifecycle and placement
- transcript and history
- resumability
- approvals and waiting
- child sessions and delegated work

That makes it hard to answer simple product questions such as:

- is this thing still running?
- can I reopen it later?
- what changed after it moved runtimes?
- which durable record should a browser or mobile UI inspect?

The run/session split gives Fireline a cleaner product story.

## What A Run Is

A run is the product object that represents active or recently active work.

It should answer:

- what is currently executing?
- where is it placed?
- what workspace and profile is it using?
- is it running, waiting, completed, failed, or cancelled?
- can it be moved, stopped, resumed, or retried?

At the product layer, `run` is the main action surface.

Examples:

- start a new coding run
- move a run from local to Docker
- list blocked runs waiting for approval
- stop a background maintenance run

## What A Session Is

A session is the durable record of the run's activity.

It should answer:

- what happened?
- what messages, prompts, and outputs occurred?
- what child sessions or peer calls were created?
- what approvals interrupted the work?
- what artifacts were produced?
- what evidence exists to replay or inspect later?

The session is the thing you reopen when the original interactive client is
gone.

## A Useful Mental Model

The cleanest mental model is:

- run = live control object
- session = durable evidence object

In many cases:

- one run creates one root session

But that run may also create:

- child sessions
- delegated peer sessions
- resumptions against the same logical session lineage

So the product layer should not pretend that a run and a session are always
identical.

## What Fireline Already Has

Fireline is not starting from zero here.

It already has:

- durable session rows in the state model
- runtime-side `SessionIndex`
- consumer-side session collections
- local `session/load` coordination and replay-oriented behavior

That means Fireline already has strong **session substrate**.

What is still missing is:

- a clearer `client.sessions` product surface
- a clearer `client.runs` product surface
- explicit mapping between run state and session lineage
- product-level lifecycle semantics that do not depend on one runtime process

## Product Responsibilities

### `client.runs`

`client.runs` should own:

- start
- stop
- cancel
- move
- retry
- placement status
- waiting status

Suggested shape:

```ts
client.runs.start(spec)
client.runs.get(runId)
client.runs.list(filter?)
client.runs.stop(runId)
client.runs.cancel(runId)
client.runs.move(runId, placement)
client.runs.retry(runId, options?)
```

### `client.sessions`

`client.sessions` should own:

- list
- get
- reopen / resume
- timeline and transcript inspection
- artifact inspection
- child session inspection

Suggested shape:

```ts
client.sessions.list(filter?)
client.sessions.get(sessionId)
client.sessions.resume(sessionId, options?)
client.sessions.timeline(sessionId)
client.sessions.artifacts(sessionId)
client.sessions.children(sessionId)
```

## Suggested Product Shapes

These are not final contracts. They show the intended contour.

```ts
type Run = {
  runId: string
  rootSessionId: string
  workspaceId?: string
  profileId?: string
  placement?: RunPlacement

  state:
    | "starting"
    | "running"
    | "waiting"
    | "completed"
    | "failed"
    | "cancelled"

  blockingRequestIds?: string[]
  createdAtMs: number
  updatedAtMs: number
}

type Session = {
  sessionId: string
  runId?: string
  parentSessionId?: string
  runtimeId?: string
  runtimeKey?: string
  nodeId?: string

  state: "open" | "paused" | "completed" | "failed" | "cancelled"

  startedAtMs: number
  endedAtMs?: number
}
```

## How They Relate To Placement

This distinction matters even more once Fireline supports distributed runtime
placement.

A run may:

- start locally
- move to Docker
- later resume on another provider-backed runtime

The durable session record should remain the stable evidence trail through those
changes.

That is why the product surface should treat runtime placement as something a
run has, not something a session is.

## Waiting And Approvals

Out-of-band approval flows strengthen the run/session split.

When a gated action occurs:

- the **run** becomes `waiting`
- the **session** records the durable evidence of why it waited
- an **approval request** becomes the serviceable object

This is the cleanest way to make delayed approvals and human intervention
restart-safe.

## Child Sessions And Delegation

This is another place where session has more nuance than run.

A single run may produce:

- child sessions for delegated work
- peer calls that create separate session lineage
- retries or resumed branches

That means `client.sessions.children(sessionId)` is likely a first-class product
need even if `client.runs.children(runId)` is not.

## Product UX Implications

In a browser or control-plane UI:

- the **Runs** page should answer "what needs attention right now?"
- the **Sessions** page should answer "what happened, and what can I reopen?"

In a CLI or editor:

- `runs` is the action-oriented view
- `sessions` is the history and replay-oriented view

That gives Fireline a much more understandable product model than exposing only
runtime ids and ACP endpoints.

## What Should Stay Below The Product Layer

These are still important, but should remain implementation-facing:

- runtime registry details
- ACP connection handles
- stream URLs
- provider instance ids
- local bootstrap files

The product layer should only expose them when an advanced operator truly needs
them.

## First-Cut Recommendation

The first product-surface delivery should be:

1. `client.runs.start/get/list`
2. `client.sessions.get/list/timeline`
3. explicit linking from run to root session
4. explicit `waiting` run state backed by durable records

That would already make Fireline feel significantly more like a durable agent
product and less like a runtime toolkit.

## What Future Slices Should Prove

Future slices in this area should prove:

- a run can be listed independently of raw runtime metadata
- a session can be reopened after the original client disconnects
- a waiting run can be serviced later and continue
- placement changes do not break the durable session story

## Non-Goals

This doc does not define:

- exact state table schemas
- final REST or TS signatures
- provider-specific placement logic
- replay internals

Those belong to technical docs and execution slices.
