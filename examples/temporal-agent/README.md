# Temporal Agent

The problem is simple: what happens when your agent needs to wait until tomorrow?

A normal JavaScript promise dies with the process. A durable wait does not. This example shows the current Fireline answer on `main`: use `ctx.awakeable(...)` to park the workflow on the state stream, then let any other process resolve the same canonical key later.

This is the imperative surface over `DurableSubscriber::Passive`, not a second workflow engine. The stream is still the source of truth. If you want the concept and proposal background, read [Awakeables](../../docs/guide/awakeables.md) and [durable-promises.md](../../docs/proposals/durable-promises.md).

## What This Example Does

1. starts a durable wait keyed by `sessionCompletionKey(sessionId)`
2. prints a JSON `waiting` record with the resume key
3. resolves that same key from another process
4. prints a JSON `resumed` record with the change-window payload

The story is intentionally concrete: an agent finishes planning, then waits for an overnight change window before continuing the rollout.

## The Core Code

This is the whole shape:

```ts
import {
  resolveAwakeable,
  sessionCompletionKey,
  workflowContext,
} from '@fireline/client'

const key = sessionCompletionKey(sessionId)
const ctx = workflowContext({ stateStreamUrl })

const wake = ctx.awakeable<{
  note: string
  openedBy: string
  window: string
}>(key)

console.log(JSON.stringify({ status: 'waiting', windowKey: wake.key }))
console.log(JSON.stringify({ status: 'resumed', resolution: await wake.promise }))

await resolveAwakeable({
  streamUrl: stateStreamUrl,
  key,
  value: {
    note: 'Ops opened the nightly change window. Continue the rollout.',
    openedBy: 'ops-oncall',
    window: 'tonight-02:00',
  },
})
```

Why this matters:

- the wait is durable because it lives on the state stream, not in process memory
- the resolver does not need to be the original process
- the key is the same canonical completion key the subscriber substrate already understands

## Run It Against A Real Fireline Session

This guide uses the current demo asset because it is the most honest local bootstrap on `main`:

```ts
agent(['npx', '-y', '@agentclientprotocol/claude-agent-acp'])
```

From the repo root:

```bash
pnpm install
pnpm --filter @fireline/cli build
cargo build --bin fireline --bin fireline-streams

export FIRELINE_BIN="$PWD/target/debug/fireline"
export FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams"
export ANTHROPIC_API_KEY="..."
```

In terminal 1, boot Fireline and copy the printed `state:` URL:

```bash
npx fireline run docs/demos/assets/agent.ts
```

Expected output excerpt:

```text
✓ fireline ready
  sandbox:   runtime:...
  ACP:       ws://127.0.0.1:...
  state:     http://127.0.0.1:7474/v1/stream/fireline-state-runtime-...
```

In terminal 2, start the durable wait:

```bash
cd examples/temporal-agent
pnpm install --ignore-workspace --lockfile=false

STATE_STREAM_URL=http://127.0.0.1:7474/v1/stream/fireline-state-runtime-demo \
SESSION_ID=session-demo \
pnpm start -- wait
```

Expected output:

```json
{"question":"Can my agent pause for an overnight change window and resume cleanly tomorrow?","resumeHint":"You can stop this process and run the same wait command later. The durable wait is keyed by sessionId, not by this PID.","sessionId":"session-demo","stateStreamUrl":"http://127.0.0.1:7474/v1/stream/fireline-state-runtime-demo","status":"waiting","story":"The planning run is done. Fireline can stay parked in the durable stream until the overnight change window opens.","windowKey":{"kind":"session","sessionId":"session-demo"}}
```

In terminal 3, resolve the same session key:

```bash
cd examples/temporal-agent

STATE_STREAM_URL=http://127.0.0.1:7474/v1/stream/fireline-state-runtime-demo \
SESSION_ID=session-demo \
OPENED_BY=release-manager \
CHANGE_WINDOW=tonight-23:00 \
pnpm start -- resolve
```

Expected output:

```json
{"question":"Can my agent pause for an overnight change window and resume cleanly tomorrow?","resolution":{"note":"Ops opened the nightly change window. Continue the rollout.","openedBy":"release-manager","window":"tonight-23:00"},"sessionId":"session-demo","stateStreamUrl":"http://127.0.0.1:7474/v1/stream/fireline-state-runtime-demo","status":"resolved","story":"An external process appended the durable completion. Any waiter on the same session key can continue now."}
```

The waiting terminal should immediately print:

```json
{"question":"Can my agent pause for an overnight change window and resume cleanly tomorrow?","resolution":{"note":"Ops opened the nightly change window. Continue the rollout.","openedBy":"release-manager","window":"tonight-23:00"},"sessionId":"session-demo","stateStreamUrl":"http://127.0.0.1:7474/v1/stream/fireline-state-runtime-demo","status":"resumed","story":"The same logical wait resumed from the durable stream after the change window opened."}
```

## Smoke Test

If you want the substrate proof without a live model session:

```bash
pnpm install
cd examples/temporal-agent
pnpm install --ignore-workspace --lockfile=false
pnpm test
```

The smoke test boots `fireline-streams`, creates a JSON state stream, runs `wait`, runs `resolve`, and asserts that the stream contains one waiting row and one resolved row for the same session key.

## What Could Go Wrong

- `STATE_STREAM_URL` must point at a real Fireline state stream.
  The waiter and resolver both operate directly on the durable stream.
- `SESSION_ID` is the resume identity.
  Change it between `wait` and `resolve` and the waiter will never see the completion.
- This example is session-scoped on purpose.
  If your real control flow needs prompt- or tool-level identity, switch to `promptCompletionKey(...)` or `toolCompletionKey(...)`.
- Duplicate completion is explicit.
  The resolver prints `already-resolved` if the same session key already has a terminal row.
