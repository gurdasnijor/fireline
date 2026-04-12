# Approvals

## Compose-time API

Approvals start in the harness spec:

```ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, trace } from '@fireline/client/middleware'

const guarded = compose(
  sandbox(),
  middleware([trace(), approve({ scope: 'tool_calls' })]),
  agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
)
```

That middleware spec is serialized in TypeScript, translated into an
`approval_gate` topology component, and instantiated by Rust in
[crates/fireline-harness/src/host_topology.rs](../../crates/fireline-harness/src/host_topology.rs).

## What the runtime does

The approval implementation lives in
[crates/fireline-harness/src/approval.rs](../../crates/fireline-harness/src/approval.rs).

At runtime the gate:

1. intercepts the prompt
2. emits a `permission_request` event into the durable state stream
3. flushes that write
4. opens a live SSE reader on the same stream
5. blocks until a matching `approval_resolved` event appears

Key functions: `emit_permission_request(...)`, `wait_for_approval(...)`,
`rebuild_from_log(...)`.

## This wait is durable

The rendezvous is the stream, not process memory. Approval state
survives process loss or restart:

- `rebuild_from_log(...)` scans the stream from the beginning and
  reconstructs pending or approved state
- `wait_for_approval(...)` then continues by live-reading the stream
  via SSE

That is the key Fireline property: approvals are resumable because the
stream is the source of truth.

## Current limitation: prompt-level fallback

The TS API lets you write `approve({ scope: 'tool_calls' })`, but the
current implementation is still prompt-level. Tool calls currently
travel as MCP-over-ACP; there is not yet a clean upstream interception
hook for individual tool dispatches.

So today:

- `scope: 'tool_calls'` means "use the approval gate, with tool-call
  vocabulary in the public API"
- the enforcement point is still the prompt path

## Resolving approvals

The **primary API** is `agent.resolvePermission(...)` on the live
`FirelineAgent`:

```ts
const agent = await compose(...).start({ serverUrl })

// Later — from the same process that holds the agent object:
await agent.resolvePermission(sessionId, requestId, {
  allow: true,
  resolvedBy: 'dashboard',
})
```

`resolvePermission` appends an `approval_resolved` event to the agent's
state stream. The harness's live SSE reader picks it up and unblocks the
gate.

## Resolving from outside the agent object

If you don't have the `FirelineAgent` — for example, you're in a
different process that observes the state stream from a separate host —
use the standalone `appendApprovalResolved` helper:

```ts
import { appendApprovalResolved } from '@fireline/client'

await appendApprovalResolved({
  streamUrl: stateStreamUrl,
  sessionId,
  requestId,
  allow: true,
  resolvedBy: 'slack-bot',
})
```

`appendApprovalResolved` is re-exported from `@fireline/client` (also
available from `@fireline/client/events`). It takes the stream URL
directly and does not require any control-plane access.

## Dashboard or bot pattern

Observe pending permissions through the DB, decide elsewhere, append a
resolution when ready:

```ts
import fireline, { appendApprovalResolved } from '@fireline/client'

const db = await fireline.db({ stateStreamUrl: agent.state.url })

db.permissions.subscribe((rows) => {
  const pending = rows.find((row) => row.state === 'pending')
  if (!pending) return

  void appendApprovalResolved({
    streamUrl: agent.state.url,
    sessionId: pending.sessionId,
    requestId: pending.requestId,
    allow: true,
  })
})
```

That is the durable approval handshake in one sentence: observe
`permission_request` on the stream, decide elsewhere, append
`approval_resolved` to the stream. The sandbox can die between the
request and the resolution — when a new one replays the stream, it sees
the approval and continues.
