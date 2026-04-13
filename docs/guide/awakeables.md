# Awakeables

Awakeables are the planned user-facing API for durable waiting in Fireline.

This guide is intentionally a stub today. The full awakeable surface is blocked on Durable Promises Phase 3 landing on `main`, so the concrete TypeScript workflow examples are not honest to publish yet.

## What This Will Be

The target shape is:

- `ctx.awakeable<T>(...)` to declare a durable wait
- `await awakeable.promise` to suspend until the matching completion arrives
- `resolveAwakeable(...)` to append that completion from another process or surface
- `sleep(...)` and similar helpers layered on top of the same completion-key substrate

The important boundary will stay the same:

- awakeables are sugar over the durable-subscriber substrate
- they are not a second workflow engine
- replay and restart still rebuild the wait from the durable stream

## Status Today

What is true on `main` right now:

- the design is documented in [docs/proposals/durable-promises.md](../proposals/durable-promises.md)
- the rollout plan is tracked in [docs/proposals/durable-promises-execution.md](../proposals/durable-promises-execution.md)
- early Rust groundwork has started, but the full user-facing wire shape this guide needs is not landed yet

That is why this page does not show `ctx.awakeable(...)` as a runnable public recipe yet.

## What To Use Today

If you need the same durable wait/resume behavior right now, use the shipped approval and durable-subscriber surfaces.

The simplest replayable example on `main` is the approval harness:

```bash
export FIRELINE_BIN="$PWD/target/debug/fireline"
export FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams"
export FQA_APPROVAL_AGENT_COMMAND="$PWD/target/debug/fireline-testy-fs"

node docs/demos/scripts/replay-fqa-approval.mjs
```

Expected output excerpt:

```json
{
  "allowVerdict": "pass",
  "denyVerdict": "pass",
  "promptLevelFallback": true,
  "publicSurfaceCoversCrashRestartSessionLoad": false
}
```

That is not the awakeable API yet. It is the current public proof that Fireline already supports the durable completion model awakeables will sit on top of.

## Read This Next

- [docs/guide/approvals.md](./approvals.md)
- [docs/guide/durable-subscriber.md](./durable-subscriber.md)
- [docs/proposals/durable-promises.md](../proposals/durable-promises.md)
- [docs/proposals/durable-promises-execution.md](../proposals/durable-promises-execution.md)

This page should expand as soon as Durable Promises Phase 3 lands on `main`.
