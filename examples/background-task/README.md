# Background Task

Sometimes the question is not "can the agent help me right now?" It is "can I hand this off and come back tomorrow?" Most agent tools still assume the user will babysit the run in one browser tab, on one machine, for the entire job.

This demo shows the Fireline version of background work. You submit a long-running task, get back a durable task identity, and walk away. The agent keeps writing its progress into the state stream. Tomorrow you can reopen that stream, inspect the full history, and continue from the same durable record instead of hoping some transient UI kept enough local state alive.

## What Happens

1. The first run provisions a sandbox and submits a long task.
2. The demo prints `taskId`, `sessionId`, and `stateStream`.
3. A later run opens `TASK_STREAM_URL` and reads the durable history back.

## The Code

```ts
const handle = await compose(sandbox({ envVars }), middleware([trace()]), agent(agentCommand))
  .start({ serverUrl, name: 'background-task' })
```

Fireline does not need a special background-job API here. The durable stream is already the long-lived record.

## Run It

```bash
pnpm --dir .. install --ignore-workspace --lockfile=false
cd examples/background-task
pnpm install
ANTHROPIC_API_KEY=... pnpm start
TASK_STREAM_URL=http://127.0.0.1:7474/streams/state/... pnpm start
```

The first run submits the work. The second run is the "check on it later" moment.
