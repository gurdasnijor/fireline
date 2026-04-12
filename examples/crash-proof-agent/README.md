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
await new SandboxAdmin({ serverUrl: primaryUrl }).destroy(first.id)
const second = await harness.start({ serverUrl: rescueUrl, stateStream })
await acp2.connection.loadSession({ sessionId, cwd: '/workspace', mcpServers: [] })
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
