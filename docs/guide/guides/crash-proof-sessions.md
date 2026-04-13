# Crash-Proof Sessions

This recipe is the current, honest Fireline answer to "what happens if the host
dies mid-task?"

The durable part of the conversation is the session plus its shared state
stream, not the first sandbox process. If you keep the same `stateStream`,
retain the original `sessionId`, and reconnect through an agent that supports
`loadSession()`, a replacement host can continue the same conversation.

Authoritative source:
[`examples/crash-proof-agent`](../../../examples/crash-proof-agent)

## What This Recipe Proves

By the end you should have:

- one durable `sessionId`
- one shared `stateStream`
- two different sandbox ids
- two completed turns in one session history

That is the product claim this guide is documenting on current `main`.

## What It Does Not Prove

This guide does not claim that:

- every ACP agent supports `loadSession()`
- every literal `kill -9` or Docker restart path is demo-green
- sandbox memory is durable by itself

The shipped example is a controlled replacement flow. It is intentionally narrow
so the restart contract stays explicit.

## Prerequisites

- Node `>=20`
- `pnpm`
- a working Rust toolchain
- the local Fireline binaries built once:
  - `fireline`
  - `fireline-streams`
  - `fireline-testy-load`

From the repo root:

```bash
pnpm install
cargo build --bin fireline --bin fireline-streams --bin fireline-testy-load
```

`fireline-testy-load` is the right first target here because it already
implements the `loadSession()` side of the story.

## 1. Start One Shared Durable-Streams Service

In terminal 1:

```bash
./target/debug/fireline-streams
```

Both hosts in this recipe must point at that same durable-streams service. If
the rescue host writes to a different backend, there is nothing to resume from.

## 2. Start A Primary Host And A Rescue Host

In terminal 2, start the primary host:

```bash
./target/debug/fireline --control-plane --port 4440 \
  --durable-streams-url http://127.0.0.1:7474/v1/stream
```

In terminal 3, start the rescue host:

```bash
./target/debug/fireline --control-plane --port 5440 \
  --durable-streams-url http://127.0.0.1:7474/v1/stream
```

The ports differ. The durable stream backend does not.

## 3. Run The Crash-Proof Example

In terminal 4:

```bash
cd examples/crash-proof-agent
pnpm install

FIRELINE_PRIMARY_URL=http://127.0.0.1:4440 \
FIRELINE_RESCUE_URL=http://127.0.0.1:5440 \
STATE_STREAM=crash-proof-demo \
AGENT_COMMAND=../../target/debug/fireline-testy-load \
pnpm start
```

Example-local env vars:

- `FIRELINE_PRIMARY_URL`
  defaults to `http://127.0.0.1:4440`
- `FIRELINE_RESCUE_URL`
  defaults to `http://127.0.0.1:5440`
- `STATE_STREAM`
  defaults to `crash-proof-${Date.now()}`
- `AGENT_COMMAND`
  defaults to `../../target/debug/fireline-testy-load`

Those are example inputs, not general Fireline package env vars. For the shared
package env/config surface, read [Environment and Config](../api/env-and-config.md).

## 4. Read The Core Handoff

This is the heart of the example:

```ts
const first = await harness.start({
  serverUrl: primaryUrl,
  name: 'crash-proof-primary',
  stateStream,
})
const acp1 = await first.connect('crash-proof-primary')
const { sessionId } = await acp1.newSession({ cwd: '/workspace', mcpServers: [] })

await acp1.prompt({
  sessionId,
  prompt: [{ type: 'text', text: 'Turn one: start auditing the repo and keep going after a crash.' }],
})

await first.stop()
await acp1.close()

const second = await harness.start({
  serverUrl: rescueUrl,
  name: 'crash-proof-rescue',
  stateStream,
})
const acp2 = await second.connect('crash-proof-rescue')

await acp2.loadSession({ sessionId, cwd: '/workspace', mcpServers: [] })
await acp2.prompt({
  sessionId,
  prompt: [{ type: 'text', text: 'Turn two: finish the audit without repeating yourself.' }],
})
```

Only two recovery details are non-negotiable:

- both `start(...)` calls use the same `stateStream`
- the replacement connection calls `loadSession({ sessionId, ... })` before the
  next prompt

If either piece changes, you are starting a fresh conversation, not resuming the
old one.

## 5. What You Must Persist

When you adapt this pattern into an app, keep these values outside the sandbox:

- the original `sessionId`
- the shared `stateStream`
- enough harness config to start a compatible replacement
- the reconnect inputs you will pass back into `loadSession(...)`
  that usually means at least `cwd` and `mcpServers`

The replacement host can change. Those values cannot.

## 6. Verify It From The State Stream

The example does not stop at `loadSession()`. It also opens a state DB on the
replacement host:

```ts
const db = await fireline.db({ stateStreamUrl: second.state.url })
```

It then waits until `db.promptRequests` shows two completed rows for the same
session before printing the final JSON summary.

Success looks like:

```json
{
  "question": "What happens when the agent crashes mid-task?",
  "sessionId": "session-...",
  "firstSandboxId": "runtime:...",
  "secondSandboxId": "runtime:...",
  "turns": [
    "Turn one: start auditing the repo and keep going after a crash.",
    "Turn two: finish the audit without repeating yourself."
  ]
}
```

What to check:

- one `sessionId`
- two different sandbox ids
- both turns present in one durable history

The example's `waitForRows(...)` helper is subscribe-based, not polling. If you
are building app code, prefer explicit `db.promptRequests.subscribe(...)` or a
live-query wrapper so the observation path stays obvious.

## 7. Demo-Safe Fallback

For rehearsal or stage work, keep the restart beat deterministic.

The safest live path today is still the same session-resume shape, but with a
resumable test agent:

- prefer `AGENT_COMMAND=../../target/debug/fireline-testy-load` in the example
- keep [`docs/demos/assets/agent-testy-load.ts`](../../demos/assets/agent-testy-load.ts)
  ready as the spec-level fallback
- if the public host-restart path is not green in rehearsal, downgrade the
  literal kill-and-resume moment to a pre-staged recording instead of
  improvising on stage

That is the same operational discipline the demo operator script uses: the
recovery pattern is real, but the show path still needs a deterministic fallback
when restart confidence is not green.

## 8. Adaptation Checklist

- provision the replacement host against the same durable-streams backend
- start the replacement harness with the same `stateStream`
- reconnect over the new ACP endpoint
- call `loadSession()` before any follow-up prompt
- observe completion from the durable stream, not from in-process memory

## Next Steps

- [First Local Agent](./first-local-agent.md) for the basic local run loop
- [Long Durable Waits](./long-durable-waits.md) for resume-friendly wait patterns
- [Sessions and ACP](../concepts/sessions-and-acp.md) for the identity model
- [`@fireline/state`](../api/state.md) for the observation surface
