# Multi-Agent Team

Real product teams do not ask one agent to do everything. They stage work, split specialist review across a small swarm, and keep a few named roles available for direct handoff when the workflow needs it. Most stacks make you bolt those shapes together yourself with process managers, queue glue, and ad hoc routing.

This example shows the Fireline version of that team topology. One shared state stream holds an ordered handoff (`pipe`), a parallel reviewer swarm (`fanout`), and a pair of named specialists ready for peer routing (`peer` plus middleware `peer()`). The host app still owns the workflow, but the deployment shape is now part of the product surface instead of hidden scaffolding.

## What This Example Shows

1. `pipe(researcher, writer)` starts the ordered stages on one durable history.
2. `fanout(reviewer, { count: 3 })` spins up three parallel reviewers against that same stream.
3. `peer(coordinator, approver)` starts named specialists together, while middleware `peer()` enables direct agent-to-agent routing when you want it.
4. `fireline.db(...)` watches the shared stream so the host can read the whole team as one deployment.

## The Code

```ts
const stageHandles = await pipe(researcher, writer).start({
  serverUrl,
  stateStream: sharedStateStream,
})
const reviewerHandles = await fanout(reviewer, { count: reviewerCount }).start({
  serverUrl,
  stateStream: sharedStateStream,
  name: reviewerBaseName,
})
const specialistHandles = await peer(coordinator, approver).start({
  serverUrl,
  stateStream: sharedStateStream,
})
```

That is the topology claim in one screenful: ordered stages, parallel copies, and named peer-aware roles can all live on the same Fireline deployment.

The rest of the file keeps the host-side workflow explicit. It prompts the researcher first, fans the research out to three reviewers in parallel, asks the coordinator for the peer-level team view, hands the combined output to the writer, and finally asks the named approver for the go/no-go call. The example reads the final text back from `db.promptRequests` and `db.chunks`, so the shared durable state remains the single record of what the team did.

## Run It

This example expects a Fireline host at `http://127.0.0.1:4440` and durable-streams at `http://127.0.0.1:7474/v1/stream`.

For a deterministic local smoke, use the repo's built test agent:

```bash
cargo build --bin fireline --bin fireline-streams --bin fireline-testy
./target/debug/fireline-streams
./target/debug/fireline --control-plane --port 4440   --durable-streams-url http://127.0.0.1:7474/v1/stream

cd examples/multi-agent-team
pnpm install --ignore-workspace --lockfile=false
TEAM_COMMAND_MODE=testy AGENT_COMMAND=../../target/debug/fireline-testy pnpm start
```

That smoke path proves the topology and durable-state wiring. The output will echo the prompts back, but you will still see one shared deployment with staged roles, reviewer fanout, and named peer-ready specialists.

For a richer run, leave `TEAM_COMMAND_MODE` unset and point `AGENT_COMMAND` at a real ACP model such as `npx -y @agentclientprotocol/claude-agent-acp`.

## Why These Three Helpers Matter Together

- `pipe(...)` is the right fit when the application should control the handoff and preserve order.
- `fanout(...)` is the right fit when the same prompt shape should run across identical workers.
- `peer(...)` is the right fit when specific named agents may need direct discovery and routing later.

Most real agent products need all three at different moments. This example keeps them in one deployment so you can copy the shape instead of rebuilding it from scratch.
