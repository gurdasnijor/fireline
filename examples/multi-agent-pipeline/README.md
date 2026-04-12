# Multi-Agent Pipeline

> A topology demo: researcher → reviewer → writer, all coordinated through one durable stream and visible through one `@fireline/state` subscription.

## Why this is uniquely Fireline

Most multi-agent systems hide orchestration behind a scheduler service or opaque server-side thread graph. Fireline’s claim is stronger:

- the topology is data: `pipe(researcher, reviewer, writer)`
- each stage is still just a normal harness
- one shared durable stream carries all turns and lineage
- one observer sees the whole pipeline without custom fan-in APIs

The magic is not “three agents in a row”. The magic is **the stream is the coordination layer**.

## What this example shows

```ts
const handles = await pipe(researcher, reviewer, writer).start({
  serverUrl: 'http://127.0.0.1:4440',
  name: 'demo-pipeline',
})
```

Then:

1. researcher writes its output
2. reviewer reads that output from the same stream and validates it
3. writer reads the reviewed output from the same stream and produces a final draft
4. one `createFirelineDB({ stateStreamUrl })` subscription sees every turn

## Run it

```bash
cargo build -q -p fireline --bin fireline --bin fireline-testy-prompt
pnpm --dir .. install --ignore-workspace --lockfile=false
cd examples/multi-agent-pipeline
pnpm install
pnpm start
```

The demo uses `fireline-testy-prompt` so you can see the exact handoff text. Swap in a real ACP agent command later and the topology story stays the same.
