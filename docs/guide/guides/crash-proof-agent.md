# Crash-Proof Agent

You want the smallest demo that proves a Fireline session can survive host
replacement.

This guide uses the shipped
[`examples/crash-proof-agent`](../../../examples/crash-proof-agent) example. The
core path is:

1. start a session on host A
2. persist its state to a shared durable stream
3. stop host A's sandbox
4. start a replacement sandbox on host B with the same `stateStream`
5. call `loadSession()` with the original `sessionId`
6. continue the same conversation

That is the demo-ready recovery story on current `main`.

What this guide does **not** claim:

- arbitrary `kill -9` process death is fully demo-green from the public path
- Docker stop/start restart semantics are clean on every target
- the helper code in the example is the final polished app-facing API

For the broader restart and `session/load` design, read
[Sessions and ACP](../concepts/sessions-and-acp.md),
[Long Durable Waits](./long-durable-waits.md), and
[docs/state/session-load.md](../../state/session-load.md).

## What You'll Prove

By the end of this recipe you should have:

- one durable `sessionId`
- one shared `stateStream`
- two different sandbox ids
- two completed prompt requests in the same session history

That is the point of the example. The durable identity is the stream-backed
session, not the first sandbox process.

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

## 1. Start One Shared Durable-Streams Service

In terminal 1:

```bash
./target/debug/fireline-streams
```

This recipe uses one durable-streams service for both hosts. The replacement
host only works if both control planes point at the same stream backend.

## 2. Start Two Fireline Hosts

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

Both hosts talk to the same durable-streams service. That shared stream is the
handoff point.

## 3. Run The Example

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

Recipe-owned env vars in this example:

- `FIRELINE_PRIMARY_URL`
  Defaults to `http://127.0.0.1:4440`
- `FIRELINE_RESCUE_URL`
  Defaults to `http://127.0.0.1:5440`
- `STATE_STREAM`
  Defaults to `crash-proof-${Date.now()}`
- `AGENT_COMMAND`
  Defaults to `../../target/debug/fireline-testy-load`

Those are example inputs, not general Fireline package env vars. The package
env/config surface itself is documented in
[Environment and Config](../api/env-and-config.md).

## 4. Read The Output

Success looks like this shape:

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

- `sessionId` is singular: the same session is reused after handoff
- `firstSandboxId` and `secondSandboxId` differ: the runtime changed
- `turns` contains both prompts in one session history

If you get that result, the replacement host loaded the original session from
durable evidence and continued it.

## 5. The Core Code Path

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

The two details that matter are:

- both `start(...)` calls use the same `stateStream`
- the second ACP connection calls `loadSession({ sessionId, ... })`

Without those two pieces, this is just a new sandbox and a new conversation.

## 6. Why The State DB Is Still In The Loop

The example also opens a state DB on the second host:

```ts
const db = await fireline.db({ stateStreamUrl: second.state.url })
```

It then waits until `db.promptRequests` shows two completed rows for the same
session before printing the final JSON summary.

Current implementation detail:

- the example uses `waitForRows(...)` from
  [`examples/shared/wait.ts`](../../../examples/shared/wait.ts)
- that helper is subscribe-based, not polling
- if you are adapting this pattern into app code, prefer direct
  `db.promptRequests.subscribe(...)` or `useLiveQuery(...)` so the observation
  flow stays explicit

The durable stream is the evidence. The helper only gates when the demo prints.

## What This Recipe Proves Today

- A replacement host can reprovision the same harness against the same durable
  stream.
- A resumable agent (`fireline-testy-load`) can accept `loadSession()` on the
  replacement host.
- The session history remains observable through `fireline.db(...)`.
- Stopping the first sandbox does not delete the durable session record.

## What It Does Not Prove

- A literal OS-level crash or `kill -9` from the public operator path.
- Full Docker restart safety across the local Tier A flow.
- That every ACP agent implements `session/load`; this recipe depends on
  `fireline-testy-load`.

For the honest status on the harsher restart path, see:

- [docs/reviews/smoke-tier-a-local-docker-2026-04-12.md](../../reviews/smoke-tier-a-local-docker-2026-04-12.md)
- [docs/reviews/fqa-approval-session-2026-04-12.md](../../reviews/fqa-approval-session-2026-04-12.md)

## How To Adapt This Pattern

When you take this out of the demo and into real app code, keep these parts:

- give replacement launches the same `stateStream`
- retain the original `sessionId`
- reconnect through the new ACP endpoint
- call `loadSession()` before sending follow-up prompts
- observe the resumed run through the state stream, not sandbox memory

The supporting references:

- [Compose and Start](../compose-and-start.md)
- [Observation](../observation.md)
- [Environment and Config](../api/env-and-config.md)
- [`@fireline/state`](../api/state.md)
