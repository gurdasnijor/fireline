# Observation Model

Most agent products get observation wrong in the same way:

- the agent does work in one process
- the UI polls another process
- someone writes glue code to merge partial status into something a user can read

That is why dashboards drift out of sync, approvals feel bolted on, and crash recovery destroys the operator view just when you need it most.

Fireline does not treat observation as a sidecar feature. It treats observation as a **stream-derived model** built from the same durable state that powers approvals, replay, and resume.

## The Core Idea

Fireline's observation plane is not "read the sandbox memory."

It is:

1. append durable state to the stream
2. materialize that stream into live collections
3. subscribe to those collections

So when you call `fireline.db(...)`, you are not opening a custom status API owned by one running sandbox. You are opening a live view over the durable session stream.

That distinction is the whole model.

## The Three Planes

Observation makes the most sense when you keep the planes separate:

- **control plane**
  Provisions sandboxes and starts harnesses
- **session plane**
  Sends ACP requests such as `session/new`, `session/prompt`, and `session/load`
- **observation plane**
  Watches the durable stream and materializes it into queryable state

You need all three, but they do different jobs.

That is why a dashboard can observe a session without being the thing that opened the ACP connection, and why a bot can react to a pending approval without reaching into sandbox memory.

## What `fireline.db(...)` Gives You

On current `main`, `fireline.db(...)` hoists four live collections onto the returned DB:

- `db.sessions`
- `db.promptRequests`
- `db.permissions`
- `db.chunks`

They come from `@fireline/state`, but `@fireline/client` gives you the simplest entry point:

```ts
import fireline from '@fireline/client'

const db = await fireline.db({ stateStreamUrl: handle.state.url })

db.permissions.subscribe((rows) => {
  const pending = rows.filter((row) => row.state === 'pending')
  console.log(pending.map((row) => row.requestId))
})
```

That is the observation contract in one screen:

- the agent does work
- Fireline appends state to the stream
- the DB projection updates
- subscribers react

No polling loop. No "ask the sandbox what happened" endpoint.

## Why This Is A Materialized View, Not The Source Of Truth

The durable stream is the source of truth.

The collections are a **materialized view** over that stream.

That means:

- if the UI disconnects, the truth still exists
- if the sandbox dies, the truth still exists
- if you rebuild the projection, the truth is still reconstructable from the stream

This is stronger than "we persisted some logs." It means a fresh reader can replay the same durable facts and recover the same observable story.

That is why observation in Fireline is tied to durability instead of treated as a separate analytics feature.

## What The Rows Represent

The live collections are not arbitrary UI state. They are compact read models over ACP-shaped agent activity.

The useful mental model is:

- `sessions`
  Which ACP sessions exist and what state they are in
- `promptRequests`
  The lifecycle of each prompt request inside a session
- `permissions`
  Durable approval requests and their resolutions
- `chunks`
  Incremental `session/update`-style activity for one request, including tool-call-related updates

These views are keyed by canonical ACP identifiers such as `SessionId`, `RequestId`, and `ToolCallId`, not by a second Fireline-only identity system.

That is what makes dashboards, approvals, and replay line up cleanly.

## Subscribe, Do Not Poll

The right way to think about Fireline observation is "subscribe to a changing model," not "ask for status every few seconds."

For simple Node or browser code, the collections expose `.subscribe(...)`.

For React, the normal pattern is `useLiveQuery`:

```tsx
const sessions = useLiveQuery((q) => q.from({ s: db.sessions }), [db])
const prompts = useLiveQuery((q) => q.from({ t: db.promptRequests }), [db])
const approvals = useLiveQuery((q) => q.from({ p: db.permissions }), [db])
```

That is the same pattern the live-monitoring example uses.

This matters because polling always asks the wrong question:

- "what is the status now?"

Observation in Fireline asks the better question:

- "what durable state changes have happened, and what does the current projection say?"

## Observation Is Useful Outside The UI

It is easy to hear "live collections" and think "front-end only."

That undersells the model. The same observation plane is useful for:

- dashboards
- approval brokers
- Slack or Telegram notifiers
- admin bots
- orchestration logic waiting for a durable condition

The stream-derived view is the shared public surface for all of those consumers.

That is why Fireline can replace a pile of per-feature glue code with one observation story.

## How This Relates To Session And Approval

Observation is not separate from the durable workflow story. It is how outside systems see that workflow.

For example:

- the session plane sends `session/prompt`
- the approval gate writes a durable permission event
- the observation plane materializes that event into `db.permissions`
- an external approver reacts and resolves it
- the session continues

The approval UI does not need a privileged hook into the conductor. It only needs to observe the durable state and write the matching resolution.

That same pattern shows up again in dashboards, crash recovery, and long waits.

## Agent Plane Vs Infrastructure Plane

The user-facing observation surface is intentionally agent-plane first.

That means the main collections are about:

- sessions
- prompt requests
- permission state
- chunked session updates

Infrastructure records such as host/runtime bookkeeping are a different concern. Fireline may materialize those for admin use, but that is not the primary app-facing observation model.

The reason for the split is simple:

- app developers usually care about "what is the agent doing?"
- operators sometimes care about "which host is serving it?"

Those are related questions, but they are not the same plane.

## A Concrete Example

The live-monitoring example shows the intended shape well:

```ts
const db = await fireline.db({ stateStreamUrl })

const sessions = useLiveQuery((q) => q.from({ s: db.sessions }), [db])
const turns = useLiveQuery((q) => q.from({ t: db.promptRequests }), [db])
const approvals = useLiveQuery((q) => q.from({ p: db.permissions }), [db])
const chunks = useLiveQuery((q) => q.from({ c: db.chunks }), [db])
```

That UI does not talk to the sandbox directly to ask:

- "are you still alive?"
- "do you have any pending permissions?"
- "how many tool calls happened?"

It reads the projected model instead.

That is the operational win: one observation surface, many consumers.

## Gotchas

- Do not treat `fireline.db()` as the source of truth.
  The durable stream is the truth; the DB is the live projection.
- Do not poll the sandbox for state you already have in the stream.
  Subscribe to the model instead.
- Do not mix up the session plane and the observation plane.
  ACP is for talking to the agent; `fireline.db()` is for observing durable state.
- Do not assume every infra record belongs in the app-facing observation surface.
  Fireline keeps the main observation model focused on agent-plane state.

## Read This Next

- [Observation](../observation.md)
- [Durable Streams](./durable-streams.md)
- [Sessions and ACP](./sessions-and-acp.md)
- [Live Monitoring example](../../../examples/live-monitoring/index.ts)
- [docs/proposals/unified-materialization.md](../../proposals/unified-materialization.md)
