# Crash-Proof Agent

Most agent demos hide the ugly part. The agent looks great right up until the process dies, the VM restarts, or the laptop lid closes. Then the conversation is gone and the user is back to copy-pasting context into a brand-new session.

This demo shows the opposite story. Fireline treats the durable stream as the session's memory, not the sandbox process. You start a task, kill the first sandbox, bring up a replacement on another Fireline host, and continue the same conversation without losing the unfinished work.

## What Happens

1. A session starts on one Fireline host.
2. The demo deliberately destroys that sandbox mid-run.
3. A second host provisions a replacement with the same `stateStream`.
4. `loadSession()` resumes the conversation from the stream-backed log.

## The Code

```ts
const first = await harness.start({ serverUrl: primaryUrl, stateStream })
await first.stop()
const second = await harness.start({ serverUrl: rescueUrl, stateStream })
const acp2 = await second.connect('crash-proof-rescue')
await acp2.loadSession({ sessionId, cwd: '/workspace', mcpServers: [] })
```

That is the core product claim: the restart path is ordinary provisioning plus ordinary ACP session load, because the session never lived inside the crashed box in the first place.

## Run It

```bash
pnpm --dir .. install --ignore-workspace --lockfile=false
cd examples/crash-proof-agent
pnpm install
FIRELINE_PRIMARY_URL=http://127.0.0.1:4440 \
FIRELINE_RESCUE_URL=http://127.0.0.1:5440 \
pnpm start
```

Use two Fireline hosts that share one durable-streams service. The output shows one `sessionId`, two different sandbox ids, and one uninterrupted turn history.

## The Primitive Behind This Example

The durability story behind this demo starts with
[acp-canonical-identifiers.md](../../docs/proposals/acp-canonical-identifiers.md).
In the target architecture, the canonical `SessionId` is the durable identity
for the agent-plane conversation, so rebuild-from-log does not need to mint a
replacement id after a crash.

That same identity discipline is what makes `session/load` plus stream replay a
clean recovery path: a new host can reattach to the existing session stream,
replay the prior events, and resume the same ACP session instead of stitching
together a new synthetic conversation handle. The durable workflow proposals in
[durable-subscriber.md](../../docs/proposals/durable-subscriber.md) and
[durable-promises.md](../../docs/proposals/durable-promises.md) depend on the
same foundation.

This section is describing the conceptual substrate the demo is aiming at. It
does not claim that the generalized DurableSubscriber or durable-promises
primitive is already the literal runtime mechanism for this example today.
