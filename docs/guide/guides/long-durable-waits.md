# Long Durable Waits

You have a wait that might outlive the sandbox that started it.

Examples:

- a human approves an action three hours later
- a vendor webhook arrives tomorrow morning
- a timer wakes the workflow after a long pause
- a crash kills the current sandbox, but the session still needs to continue

For that class of problem, the only question that matters is this:

**Is the wait part of the workflow itself, or is it just external glue code
watching the workflow from the outside?**

That is the line between `ctx.awakeable(...)` and raw polling.

## The short answer

Use `ctx.awakeable(...)` when the wait is part of the durable workflow state and
must survive replay as a first-class suspension point.

Use raw polling, stream subscriptions, or helpers like `waitForRows(...)` when
you are outside the workflow and just watching Fireline from another process.

## Pick the right tool

| Situation | Use this | Why |
| --- | --- | --- |
| A workflow step must pause until someone or something resolves it later | `ctx.awakeable(...)` | The wait is recorded on the agent plane and replay reconstructs it after restart |
| A dashboard, script, or operator tool is only observing progress from the outside | stream read / `waitForRows(...)` | The watcher can come and go without becoming part of workflow correctness |
| You are resuming the same ACP session on a new host after a crash | shared `stateStream` + `loadSession()` | The durable stream already holds the session history; you do not need a second wait primitive |
| You are polling infrastructure readiness, ports, or host health | raw polling | That is an infrastructure-plane concern, not an agent-plane durable wait |

Rule of thumb:

- if losing the wait would be a correctness bug, use `ctx.awakeable(...)`
- if losing the wait would only mean rerunning a helper script, polling is fine

## What `ctx.awakeable(...)` is on current `main`

The landed surface today is the Rust workflow context added in `3b8aa7d`:

- [`crates/fireline-harness/src/workflow_context.rs`](../../crates/fireline-harness/src/workflow_context.rs)
- [`crates/fireline-harness/src/awakeable.rs`](../../crates/fireline-harness/src/awakeable.rs)
- [`tests/awakeable_basic.rs`](../../tests/awakeable_basic.rs)
- [`tests/awakeable_replay.rs`](../../tests/awakeable_replay.rs)

The important design point is that awakeables are not a second workflow engine.
They are imperative sugar over the passive durable-subscriber substrate.

Current Rust API:

```rust
let context = WorkflowContext::new(state_stream_url);
let approval = context.awakeable::<ApprovalDecision>(
    AwakeableKey::prompt(session_id, request_id),
);
let resolved = approval.await?;
```

Resolution is still just a completion envelope on the same durable stream:

```rust
producer.append_json(&awakeable_resolution_envelope(
    key,
    ApprovalDecision {
        allow: true,
        reviewer: "ops-oncall".to_string(),
    },
)?)
```

The replay test is the part that matters for long waits: if the completion is
already on the stream, recreating the workflow context resolves immediately
instead of re-waiting.

## When awakeables are the right choice

Reach for `ctx.awakeable(...)` when all of these are true:

- the wait belongs to the workflow's business logic
- another sandbox or process may need to resume it later
- you want the wait keyed by canonical session, request, or tool-call ids
- the completion should be durable evidence on the same stream as the rest of
  the workflow

Good fits:

- human approval inside a workflow step
- an external callback that should resume the same logical step
- a timer or reminder that must survive restart
- a multi-reviewer join where both resolutions are part of workflow state

Poor fits:

- waiting for a control plane port to open
- watching a demo until two prompt rows appear
- a local operator script that can be restarted cheaply

## What still uses raw polling today

The shipped TypeScript examples are still mostly outside the workflow context, so
they use stream-side waiting helpers instead of `ctx.awakeable(...)`.

The clean reference is [`examples/crash-proof-agent/index.ts`](../../examples/crash-proof-agent/index.ts):

```ts
const first = await harness.start({ serverUrl: primaryUrl, name: 'crash-proof-primary', stateStream })
await first.stop()

const second = await harness.start({ serverUrl: rescueUrl, name: 'crash-proof-rescue', stateStream })
await acp2.loadSession({ sessionId, cwd: '/workspace', mcpServers: [] })

const turns = await waitForRows(
  db.promptRequests,
  (rows) => rows.filter((row) => row.sessionId === sessionId && row.state === 'completed').length >= 2,
  10_000,
)
```

That is still the right tool in this example because the wait lives in the demo
driver, not inside a workflow step. The agent's durable identity is the shared
`stateStream` plus `sessionId`; the helper script is just observing.

## The pattern to avoid

Do not turn a long wait into a fragile in-memory loop inside the sandbox.

Bad mental model:

- start a sandbox
- `setInterval(...)` or busy-loop inside it for hours
- hope the process is still there when the answer arrives

Better mental model:

- put the durable fact on the stream
- let a later runtime replay that fact
- keep outer-process polling small and disposable unless the wait itself is part
  of workflow correctness

That is why the crash-proof example survives sandbox death and why awakeables
are modeled as passive durable subscribers instead of timer threads or promise
registries in memory.

## A practical decision checklist

- Is the thing you are waiting for semantically part of the workflow?
  Use `ctx.awakeable(...)`.
- Is the waiter just a test, demo harness, or operator utility?
  Use a stream read or helper like `waitForRows(...)`.
- Do you need the wait keyed by canonical ACP ids and reconstructable from
  replay?
  Use `ctx.awakeable(...)`.
- Are you just waiting for infrastructure readiness or host state?
  Raw polling is fine.

## Current status note

The durable wait substrate is ahead of the polished public app API.

What is landed today:

- Rust `WorkflowContext::awakeable<T>(...)`
- `AwakeableSubscriber` as a passive durable-subscriber profile
- replay tests proving already-resolved waits do not re-wait
- approval-specific durable waiting on the current public TypeScript surface

What is still mostly design surface:

- the broader TypeScript workflow API described in
  [`docs/proposals/durable-promises.md`](../../proposals/durable-promises.md)

So if you are writing Rust workflow logic, use awakeables now. If you are using
the public TypeScript agent examples today, expect to keep using shared
`stateStream`, `loadSession()`, and stream-side wait helpers until that broader
surface lands.

## Read this next

- [Quickstart](./quickstart.md) for the smallest working `npx fireline <spec>` path.
- [Approvals](../approvals.md) for the current public durable-wait story in TypeScript.
- [Custom middleware](./custom-middleware.md) for the passive and active subscriber patterns underneath awakeables.
- [examples/crash-proof-agent/README.md](../../examples/crash-proof-agent/README.md) for the current crash-recovery walkthrough.
