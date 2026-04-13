# Awakeables

Awakeables are Fireline's promise-shaped API for durable waiting.

The important boundary is simple: awakeables are imperative sugar over `DurableSubscriber::Passive`. They do not add a second workflow engine, a second id scheme, or a second replay path. The durable stream is still the source of truth.

## What This Does

Use an awakeable when your workflow code wants to pause inline and continue later when some outside actor resolves the matching completion key.

Good fits:

- human approval inside a larger workflow
- waiting for a webhook or vendor callback
- waiting for another Fireline process to publish a result
- joining or racing multiple durable waits in one control-flow block

What ships on `main` today:

- `workflowContext({ stateStreamUrl })` and `new WorkflowContext(...)`
- `ctx.awakeable<T>(key)`
- `resolveAwakeable({ streamUrl, key, value, traceContext? })`
- canonical key helpers: `promptCompletionKey(...)`, `toolCompletionKey(...)`, `sessionCompletionKey(...)`

## Fastest Way To Try It

The quickest smoke test is three terminals:

1. boot any Fireline spec and copy the printed `state` URL
2. start an awakeable waiter against that stream
3. resolve it from another process

### 1. Boot Fireline And Copy The State URL

```bash
npx fireline run agent.ts
```

Expected output excerpt:

```text
✓ fireline ready
  ACP:     ws://127.0.0.1:...
  state:   http://127.0.0.1:7474/v1/stream/fireline-state-runtime-...
```

Keep that process running. You will use the printed `state` URL below as `STATE_STREAM_URL`.

### 2. Start A Prompt-Scoped Awakeable

In a second terminal:

```bash
STATE_STREAM_URL=http://127.0.0.1:7474/v1/stream/fireline-state-runtime-demo \
node --input-type=module <<'EOF'
import { workflowContext } from '@fireline/client'

const ctx = workflowContext({ stateStreamUrl: process.env.STATE_STREAM_URL })
const approval = ctx.awakeable({
  kind: 'prompt',
  sessionId: 'session-demo',
  requestId: 'request-demo',
})

console.log('waiting on', approval.key)
console.log(await approval.promise)
EOF
```

Expected output:

```text
waiting on { kind: 'prompt', sessionId: 'session-demo', requestId: 'request-demo' }
```

At that point the process blocks. Internally, Fireline has already appended an `awakeable_waiting` envelope to the stream and is now watching for the matching completion.

### 3. Resolve It From Another Process

In a third terminal:

```bash
STATE_STREAM_URL=http://127.0.0.1:7474/v1/stream/fireline-state-runtime-demo \
node --input-type=module <<'EOF'
import { resolveAwakeable } from '@fireline/client'

await resolveAwakeable({
  streamUrl: process.env.STATE_STREAM_URL,
  key: {
    kind: 'prompt',
    sessionId: 'session-demo',
    requestId: 'request-demo',
  },
  value: { approved: true },
})

console.log('resolved')
EOF
```

Expected output:

```text
resolved
```

The waiting terminal should immediately unblock and print:

```text
{ approved: true }
```

This is the whole mental model:

- `ctx.awakeable(...)` writes the durable wait
- another process appends the durable completion
- the original workflow continues when the completion arrives

## The Core TypeScript Shape

This is the smallest real TypeScript example on `main`:

```ts
import { workflowContext } from '@fireline/client'
import type { RequestId, SessionId } from '@agentclientprotocol/sdk'

declare const stateStreamUrl: string
declare const sessionId: SessionId
declare const requestId: RequestId

const ctx = workflowContext({ stateStreamUrl })

const approval = ctx.awakeable<{ approved: boolean }>({
  kind: 'prompt',
  sessionId,
  requestId,
})

await sendApprovalCard({ resumeKey: approval.key })

const result = await approval.promise
if (!result.approved) {
  throw new Error('denied')
}
```

Why this is useful:

- the wait stays inline with the rest of your business logic
- the durable stream still owns the pause/resume semantics
- `approval.key` is the same canonical completion key the passive subscriber substrate uses underneath

## Resolving The Awakeable

The resolver can run in the same process, another service, a bot worker, or a webhook handler. The important thing is that it appends the matching completion to the same stream.

```ts
import { resolveAwakeable } from '@fireline/client'

await resolveAwakeable({
  streamUrl: stateStreamUrl,
  key: approval.key,
  value: { approved: true },
  traceContext: {
    traceparent: '00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01',
    tracestate: 'vendor=value',
    baggage: 'tenant=acme',
  },
})
```

That writes the canonical `awakeable_resolved` envelope. The helper rejects if the same key was already resolved.

## Canonical Keys

The current TypeScript surface uses the durable-subscriber completion key directly. Today that means three key shapes on `main`:

- prompt-scoped: `{ kind: 'prompt', sessionId, requestId }`
- tool-scoped: `{ kind: 'tool', sessionId, toolCallId }`
- session-scoped: `{ kind: 'session', sessionId }`

If you prefer helpers instead of object literals:

```ts
import {
  promptCompletionKey,
  toolCompletionKey,
  sessionCompletionKey,
} from '@fireline/client'

const promptKey = promptCompletionKey({ sessionId, requestId })
const toolKey = toolCompletionKey({ sessionId, toolCallId })
const sessionKey = sessionCompletionKey(sessionId)
```

And if you specifically want a session-scoped wait:

```ts
import { WorkflowContext } from '@fireline/client'

const ctx = new WorkflowContext({ stateStreamUrl })
const wake = ctx.sessionAwakeable<{ status: string }>(sessionId)
console.log(await wake.promise)
```

## Where The Key Comes From In A Real App

The smoke test above uses synthetic ids so you can prove the API shape quickly. In a real workflow, the key should come from real Fireline state:

- a pending approval row gives you `sessionId` and `requestId`
- a tool-level workflow gives you `sessionId` and `toolCallId`
- a session-level checkpoint gives you `sessionId`

That is the architectural point of awakeables: Fireline does not mint a second imperative id. The workflow waits on the same canonical agent-plane identity the durable-subscriber substrate already understands.

## Duplicate Resolution Is An Error

If you want idempotent resolver code, catch `AwakeableAlreadyResolvedError`:

```ts
import {
  AwakeableAlreadyResolvedError,
  resolveAwakeable,
} from '@fireline/client'

try {
  await resolveAwakeable({
    streamUrl: stateStreamUrl,
    key,
    value: true,
  })
} catch (error) {
  if (error instanceof AwakeableAlreadyResolvedError) {
    return
  }
  throw error
}
```

That matches the runtime contract: first matching completion wins for a canonical key.

## What Could Go Wrong

- `stateStreamUrl` must be the real Fireline state stream.
  Awakeables read and write directly against the durable stream; a wrong URL means the waiter never sees the completion.
- The current TypeScript surface is key-based, not step-based.
  The proposal talks about step-scoped waits and `PromptStepKey(..., StreamOffset)`, but the shipped TS API on `main` currently exposes prompt, tool, and session keys only.
- `sleep(...)` is still proposal-only.
  There is no durable timer helper on the public TypeScript surface yet.
- A regular JavaScript timeout is not a durable timeout.
  `Promise.race([approval.promise, new Promise((resolve) => setTimeout(resolve, 5_000))])` may be useful for a local smoke, but it does not survive process death the way a future durable timer helper would.
- The promise resolves only when the stream sees the matching completion.
  If the stream closes cleanly without a matching `awakeable_resolved`, the waiter rejects.

## Relationship To Approvals

Approvals are the easiest bridge to this model.

The current approval surface on `main` is still:

- `approve({ scope: 'tool_calls' })`
- `handle.resolvePermission(...)`
- `appendApprovalResolved(...)`

Awakeables sit one layer below that naming. The paused approval is a durable wait keyed by canonical ids; the approval resolution is just the matching durable completion. That is why the durable-promises proposal describes awakeables as imperative sugar over `DurableSubscriber::Passive`, not as a separate primitive.

If you want the current end-to-end approval proof, use:

- [docs/guide/approvals.md](./approvals.md)
- [docs/demos/fqa-approval-demo-capture.md](../demos/fqa-approval-demo-capture.md)

## What Is Still Ahead Of `main`

The design docs are intentionally a little ahead of the shipped TypeScript surface.

Still proposal-level today:

- step-scoped awakeables derived from durable stream offsets
- durable `sleep(...)` and timeout helpers
- the fuller Promise-composition ergonomics described in `docs/proposals/durable-promises.md`

The guide above is limited to what has actually landed in `packages/client/src/workflow/`.

## Read This Next

- [Durable subscribers](./durable-subscriber.md)
- [Approvals](./approvals.md)
- [docs/guide/concepts/durable-promises.md](./concepts/durable-promises.md)
- [docs/guide/rfcs/durable-promises.md](./rfcs/durable-promises.md)
- [docs/proposals/durable-promises.md](../proposals/durable-promises.md)
