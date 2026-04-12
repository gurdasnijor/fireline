# Multi-Agent Team

The question is not whether one model can answer one prompt. The question is whether a team of agents can divide work the way a human team would: one gathers facts, one turns them into something shippable, and you can still understand what happened afterward.

This demo shows the Fireline version of that story. A researcher agent works first. A writer agent picks up the researcher’s output from the same durable stream and turns it into a final document. The agents do not share memory and they do not call a hidden coordinator API. They coordinate through the log, so the collaboration stays inspectable.

## What Happens

1. `pipe(researcher, writer)` provisions two separate harnesses.
2. The researcher writes its findings into the shared stream.
3. The writer reads that output and turns it into a final draft.
4. One `@fireline/state` view sees the whole handoff.

## The Code

```ts
const handles = await pipe(researcher, writer).start({
  serverUrl,
  name: 'multi-agent-team',
})
```

That line is the whole topology claim: multiple agents, one shared durable history, ordinary handles on both sides.

## Run It

```bash
pnpm --dir .. install --ignore-workspace --lockfile=false
cd examples/multi-agent-team
pnpm install
pnpm start
```

Swap in a real ACP agent if you want richer behavior. The architecture does not change: the handoff still happens through the durable stream, and the monitoring surface stays the same.
