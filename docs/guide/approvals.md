# Approvals

Approvals are how you let an agent move quickly without giving it silent permission to do risky work.

In Fireline, an approval is not an in-memory callback. It is a durable workflow:

1. the agent hits an approval gate
2. Fireline writes a pending permission row to the state stream
3. some outside surface renders that request to a human or automation
4. that outside surface writes a resolution back
5. the original run continues from the same durable state

That is why approvals survive restarts and reconnects. The stream is the source of truth.

## What This Does

Use approvals when you want the agent to propose an action but wait before it actually happens. Common examples:

- delete or overwrite files
- post to an external system
- apply code changes after review
- route a human decision through Slack, Telegram, or a webhook listener

The shipped public surface today is:

- `approve({ scope: 'tool_calls' })` in your composed spec
- `fireline.db({ stateStreamUrl })` to observe pending approvals
- `handle.resolvePermission(...)` if your app owns the live agent handle
- `appendApprovalResolved(...)` if the approver runs somewhere else

## The Approval Model: Durable And Passive

Approvals are the reference case for Fireline's passive durable workflow model.

What "passive" means here:

- the approval gate writes a durable request and waits
- the gate does not decide where humans see that request
- your app chooses the rendering surface: webhook, dashboard, Telegram, or something else

That split is useful because it keeps one durable approval key while letting you change the UI later.

## Fastest Way To Try It

The public replay from the FQA-4 capture is the quickest end-to-end proof on `main`:

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
  "promptLevelFallback": true,
  "publicSurfaceCoversCrashRestartSessionLoad": false
}
```

What that replay proves:

- the approval request appears on the durable stream
- an external resolver can allow the request and the prompt continues
- an external resolver can deny the request and no success chunk is emitted

If you want the smaller in-process example instead, see:

- [examples/approval-workflow/README.md](../../examples/approval-workflow/README.md)
- [examples/approval-workflow/index.ts](../../examples/approval-workflow/index.ts)

## Minimal Compose Shape

This is the approval middleware shape to remember:

```ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, trace } from '@fireline/client/middleware'

const handle = await compose(
  sandbox(),
  middleware([trace(), approve({ scope: 'tool_calls' })]),
  agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
).start({ serverUrl: 'http://127.0.0.1:4440', name: 'approval-demo' })
```

What happens after that:

- Fireline pauses when the approval gate matches
- a pending permission row appears on `handle.state.url`
- your approver resolves the request
- the original run resumes from the same durable workflow

## Choosing The Resolution API

There are two normal ways to resolve an approval.

### Same Process: `handle.resolvePermission(...)`

Use this when your app started the agent and still holds the live `FirelineAgent`.

```ts
await handle.resolvePermission(sessionId, requestId, {
  allow: true,
  resolvedBy: 'review-dashboard',
})
```

This is the cleanest path for:

- local apps
- in-process dashboards
- examples like `examples/approval-workflow`

### External Process: `appendApprovalResolved(...)`

Use this when the approver runs outside the process that launched the agent.

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

This is the right fit for:

- a webhook receiver
- a bot worker
- a separate admin service

## Observing Pending Approvals

The simplest observation surface is the state DB:

```ts
import fireline from '@fireline/client'

const db = await fireline.db({ stateStreamUrl: handle.state.url })

db.permissions.subscribe((rows) => {
  const pending = rows.find((row) => row.state === 'pending')
  if (!pending) return

  console.log({
    sessionId: pending.sessionId,
    requestId: pending.requestId,
    title: pending.title,
  })
})
```

That gives you one place to:

- render an approval card
- notify a human
- hand the request to an automation rule

## What Could Go Wrong

- `approve({ scope: 'tool_calls' })` still uses a prompt-level fallback today.
  The public API already says "tool calls", but the current enforcement point is still the prompt path. The FQA replay reports this honestly as `promptLevelFallback: true`.
- Denied approvals still surface a generic `Internal error`.
  The durable decision is correct, but the user-facing denial message is still rough.
- The public replay does not cover the old `kill -9` + restart + `session/load` QA leg.
  The approval substrate is durable, but that exact crash-proof operator story is still tracked separately from the surfaced docs path.
- Approval rendering is your responsibility.
  The gate writes durable state and waits; you still need a surface that turns that pending row into a human decision.

## Where To Send The Approval Request

Two common next steps:

- [Durable subscribers](./durable-subscriber.md)
  Use an active delivery profile such as `webhook(...)` when you want Fireline to push the event out to another system.
- [Telegram](./telegram.md)
  Use `telegram(...)` when you want the approval UI to live in chat.

If you want the low-level design detail behind those delivery profiles, see:

- [docs/proposals/durable-subscriber.md](../proposals/durable-subscriber.md)

## Deeper References

- [docs/demos/fqa-approval-demo-capture.md](../demos/fqa-approval-demo-capture.md)
- [examples/approval-workflow/README.md](../../examples/approval-workflow/README.md)
- [docs/guide/middleware.md](./middleware.md)
- [Awakeables](./awakeables.md) for the shipped `ctx.awakeable<T>(...)` / `resolveAwakeable(...)` surface layered on the same durable mechanism
- [docs/proposals/durable-promises.md](../proposals/durable-promises.md) for the broader design and future additive sugar
