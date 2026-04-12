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

That middleware spec is serialized in TypeScript, translated into an `approval_gate` topology component, and instantiated by Rust in [crates/fireline-harness/src/host_topology.rs](../../crates/fireline-harness/src/host_topology.rs).

## What the runtime does

The actual approval implementation lives in [crates/fireline-harness/src/approval.rs](../../crates/fireline-harness/src/approval.rs).

At runtime the gate:

1. intercepts the prompt
2. emits a `permission_request` event into the durable state stream
3. flushes that write
4. opens a live SSE reader on the same stream
5. blocks until a matching `approval_resolved` event appears

The relevant pieces are:

- `emit_permission_request(...)`
- `wait_for_approval(...)`
- `rebuild_from_log(...)`

## This wait is durable

The rendezvous is the stream, not process memory.

That means approval state can survive process loss or restart:

- `rebuild_from_log(...)` scans the stream from the beginning and reconstructs pending or approved state
- `wait_for_approval(...)` then continues by live-reading the stream via SSE

That is the key Fireline property here: approvals are resumable because the stream is the source of truth.

## Current limitation: prompt-level fallback

The TS API lets you write `approve({ scope: 'tool_calls' })`, but the current implementation is still prompt-level.

Why:

- tool calls are currently traveling as MCP-over-ACP
- there is not yet a clean upstream interception hook for individual tool dispatches

This is documented in the big module comment at the top of [crates/fireline-harness/src/approval.rs](../../crates/fireline-harness/src/approval.rs), and the fallback mapping is visible in [packages/client/src/sandbox.ts](../../packages/client/src/sandbox.ts).

So today:

- `scope: 'tool_calls'` means “use the approval gate, with tool-call vocabulary in the public API”
- the enforcement point is still the prompt path

## Resolving approvals from outside the sandbox

The helper for appending a resolution event already exists:

- [packages/client/src/events.ts](../../packages/client/src/events.ts)

```ts
import { appendApprovalResolved } from '@fireline/client/events'

await appendApprovalResolved({
  streamUrl: handle.state.url,
  sessionId,
  requestId,
  allow: true,
  resolvedBy: 'dashboard',
})
```

Important:

- this helper is **not** re-exported from [packages/client/src/index.ts](../../packages/client/src/index.ts)
- import it from `@fireline/client/events`, not `@fireline/client`

## Dashboard or bot pattern

Observe pending permissions through the DB, then append a resolution when the human decides:

```ts
import { createFirelineDB } from '@fireline/state'
import { appendApprovalResolved } from '@fireline/client/events'

const db = createFirelineDB({ stateStreamUrl: handle.state.url })
await db.preload()

db.collections.permissions.subscribe((rows) => {
  const pending = rows.find((row) => row.state === 'pending')
  if (!pending) return

  void appendApprovalResolved({
    streamUrl: handle.state.url,
    sessionId: pending.sessionId,
    requestId: pending.requestId,
    allow: true,
  })
})
```

That is the durable approval handshake in one sentence:

observe `permission_request` on the stream, decide elsewhere, append `approval_resolved` to the stream.
