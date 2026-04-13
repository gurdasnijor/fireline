# Background Task

Some agent work is not a chat-tab moment. It is the thing you start before
lunch, let run while you are away, and inspect when you get back.

This example shows the Fireline shape for that workflow:

- launch the task normally
- keep the durable state stream URL
- reopen that stream later and read back the same task state

The important product point is that you do not need a special "background jobs"
API. The durable stream is already the long-lived record.

## What This Example Shows

1. The first run starts a task and prints `taskId`, `sessionId`, and
   `stateStream`.
2. The second run uses `TASK_STREAM_URL` to reopen that same durable history.
3. The readback view shows session state, prompt requests, and the latest agent
   text without reconnecting to the original process.

## The Code

```ts
const handle = await compose(
  sandbox({
    provider: 'local',
    envVars: process.env.ANTHROPIC_API_KEY
      ? { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY }
      : undefined,
  }),
  middleware([trace()]),
  agent(agentCommand),
).start({ serverUrl, name: 'background-task' })

const acp = await handle.connect('background-task')
const { sessionId } = await acp.newSession({ cwd: '/workspace', mcpServers: [] })

await acp.prompt({
  sessionId,
  prompt: [{ type: 'text', text: taskPrompt }],
})

console.log({
  taskId: handle.id,
  sessionId,
  stateStream: handle.state.url,
})
```

The "check back later" half is just:

```ts
const db = await fireline.db({ stateStreamUrl: process.env.TASK_STREAM_URL })
```

That is the product message. Fireline does not bolt on a second job-status API.
The state stream is the durable read model.

## Run It

Prerequisites:

- a Fireline host reachable at `FIRELINE_URL` or `http://127.0.0.1:4440`
- Node `>=20`
- `pnpm`
- either:
  - `ANTHROPIC_API_KEY` for the default `claude-agent-acp` path, or
  - a local ACP test agent via `AGENT_COMMAND`

Install dependencies:

```bash
pnpm install
cd examples/background-task
pnpm install --ignore-workspace --lockfile=false
```

Fast deterministic smoke on current `main`:

```bash
AGENT_COMMAND=/absolute/path/to/fireline/target/debug/fireline-testy-prompt \
pnpm start
```

That prints a `stateStream` URL. Run the same example again with it:

```bash
TASK_STREAM_URL=http://127.0.0.1:7474/v1/stream/fireline-state-runtime-... \
pnpm start
```

Default product-shaped run with Claude Agent ACP:

```bash
ANTHROPIC_API_KEY=... \
pnpm start
```

## Why This Is The Right Story

The demo is no longer "fire and forget" in the abstract. It is the concrete
workflow teams actually want:

- start a repo audit before you step away
- keep the durable task identity
- reopen the state stream later from another shell, dashboard, or worker

That is a better product story than "background tasks" because it tells the
reader exactly what Fireline is buying them: durable continuity across time and
process boundaries.

## Honest Notes On Current `main`

- This example avoids `secretsProxy()`.
  `mono-4t4` is still open, so the current safe path for the demo is direct
  `sandbox({ envVars })` or a local test-agent override.
- The deterministic smoke path uses `fireline-testy-prompt`.
  That makes the readback stable and testable. The default Claude path is the
  one that turns this into a real long-running analysis workflow.
- The example reads durable state after the run, not while a live prompt is
  still mid-flight.
  The point here is the durable check-back pattern, not a resumable operator
  console.

## Read Next

- [Observation](../../docs/guide/observation.md)
- [Durable Streams](../../docs/guide/concepts/durable-streams.md)
- [Sessions and ACP](../../docs/guide/concepts/sessions-and-acp.md)
