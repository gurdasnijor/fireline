# Background Task

Sometimes the question is not "can the agent help me right now?" It is "can I hand this off and come back tomorrow?" Most agent tools still assume the user will babysit the run in one browser tab, on one machine, for the entire job.

This demo shows the Fireline version of background work. You submit a long-running task, get back a durable task identity, and walk away. The agent keeps writing its progress into the state stream. Tomorrow you can reopen that stream, inspect the full history, and continue from the same durable record instead of hoping some transient UI kept enough local state alive.

## What Happens

1. The first run provisions a sandbox and submits a long task.
2. The demo prints `taskId`, `sessionId`, and `stateStream`.
3. A later run opens `TASK_STREAM_URL` and reads the durable history back.

## The Code

```ts
const handle = await compose(
  sandbox(),
  middleware([
    trace(),
    secretsProxy({
      ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' },
    }),
  ]),
  agent(agentCommand),
).start({ serverUrl, name: 'background-task' })
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

## The Primitive Behind This Example

The conceptual foundation here is Fireline's durable-streams substrate plus the
canonical identity model in
[acp-canonical-identifiers.md](../../docs/proposals/acp-canonical-identifiers.md).

This example does not need a named `DurableSubscriber` instance to make sense.
The underlying idea is simpler: the agent's durable state lives on the session
stream, so the runner process can come and go without becoming the source of
truth. That is the same observation model the passive durable workflow proposals
build on in [durable-subscriber.md](../../docs/proposals/durable-subscriber.md)
and [durable-promises.md](../../docs/proposals/durable-promises.md): the stream
is the durable record, and local processes are just temporary executors.

So the "background task" story is really "agent outlives the runner" semantics.
This section is pointing at the target architecture behind that story, not
claiming the full durable-promises API already exists in the runtime today.
