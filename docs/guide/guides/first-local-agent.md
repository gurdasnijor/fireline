# First Local Agent

Quickstart proves that `npx fireline` boots and exposes ACP plus a durable
state stream. This guide takes the next step: run the current frozen local demo
spec, inspect what is in that file, understand what `Ctrl+C` actually tears
down, and learn the reconnect pattern that works on current `main`.

This guide uses the real demo asset at
[`docs/demos/assets/agent.ts`](../../demos/assets/agent.ts) plus the approval
reference in
[`examples/approval-workflow/`](../../../examples/approval-workflow/README.md).

## What You'll Do

- build the local CLI and runtime binaries once
- run the demo asset with the Fireline REPL
- read the spec as `sandbox(...)`, `middleware([...])`, and `agent([...])`
- reconnect to a live session from a second terminal

## Prerequisites

- Node `>=20`
- `pnpm`
- a working Rust toolchain
- repo checkout at current `main`
- credentials for the ACP agent you plan to launch

The current demo asset launches:

```ts
agent(['npx', '-y', '@agentclientprotocol/claude-agent-acp'])
```

So the simplest path is to export the model credentials that agent expects
before you run the guide. For the broader env and secret surface, read
[Environment and Config](../api/env-and-config.md).

## 1. Build The Local Runtime Once

From the repo root:

```bash
pnpm install
pnpm --filter @fireline/cli build
cargo build --bin fireline --bin fireline-streams

export FIRELINE_BIN="$PWD/target/debug/fireline"
export FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams"
export ANTHROPIC_API_KEY="..."
```

Those binary overrides match the CLI resolution path documented in
[Environment and Config](../api/env-and-config.md).

## 2. Start The Demo Asset Locally

Run the actual frozen guide spec:

```bash
npx fireline run docs/demos/assets/agent.ts --repl
```

If the default ports are busy:

```bash
npx fireline run docs/demos/assets/agent.ts --repl --port 15443 --streams-port 17477
```

The ready banner looks like:

```text
durable-streams ready at http://127.0.0.1:17477/v1/stream

  ✓ fireline ready

    sandbox:   runtime:...
    ACP:       ws://127.0.0.1:15443/acp
    state:     http://127.0.0.1:17477/v1/stream/fireline-state-runtime-...
    session:   session-...

  Press Ctrl+C to shut down.
```

At that point the local host is up and the REPL is already attached to a fresh
ACP session.

## 3. Read The Spec File

The current demo asset is small enough to read in one glance:

```ts
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { approve, budget, peer, secretsProxy, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    budget({ tokens: 2_000_000 }),
    peer({ peers: ['reviewer'] }),
  ]),
  agent(['npx', '-y', '@agentclientprotocol/claude-agent-acp']),
)
```

Read it in three pieces:

- `sandbox({ resources: [localPath('.', '/workspace')] })`
  mounts the repo into the local sandbox as `/workspace`.
- `middleware([...])`
  is the ordered declarative behavior contract that the host lowers into the
  running pipeline.
- `agent([...])`
  is the ACP-speaking child process command Fireline launches in that sandbox.

The CLI expects the default export to be a composed harness. If the file exports
plain data or a raw object without `.start()`, `fireline run` rejects it with a
compose-based hint.

## 4. Middleware Ordering On Current `main`

The current asset order is:

```ts
trace() -> approve({ scope: 'tool_calls' }) -> budget({ tokens: 2_000_000 }) -> peer({ peers: ['reviewer'] })
```

That order matters. Fireline lowers the middleware array in order, so treat the
array as part of the spec contract, not cosmetic formatting.

What each entry is doing here:

- `trace()`
  keeps the run observable from the start.
- `approve({ scope: 'tool_calls' })`
  turns risky tool calls into durable approval checkpoints.
- `budget({ tokens: 2_000_000 })`
  makes the run-level token ceiling explicit in the same authored spec.
- `peer({ peers: ['reviewer'] })`
  enables peer MCP delegation to the named reviewer agent.

If you also need env-backed secret injection, use
[`examples/approval-workflow/index.ts`](../../../examples/approval-workflow/index.ts)
as the current reference shape:

```ts
secretsProxy({
  ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' },
})
```

That is the right surface to copy today. The frozen demo asset itself is
currently running without `secretsProxy()`.

## 5. What `Ctrl+C` Actually Does

There are two different local loops:

### `fireline run <spec> --repl`

This is the one-command convenience path. When you press `Ctrl+C`, `Ctrl+D`, or
type `/quit`, the REPL exits and the CLI tears the whole local stack down:

- the provisioned sandbox is destroyed
- the local Fireline host stops
- the child `fireline-streams` process stops

So this is the fastest first run, but not the right shape if you want to close
the REPL and come back later.

### `fireline run <spec>` plus `fireline repl`

This is the reconnectable path.

Keep the host alive in terminal 1:

```bash
npx fireline run docs/demos/assets/agent.ts
```

Then attach from terminal 2:

```bash
npx fireline repl
```

In this split setup, `Ctrl+C` in the REPL terminal only exits the REPL. The
host in terminal 1 keeps running until you stop that process separately.

## 6. How To Reconnect

If the host is still running and the downstream agent advertises session resume
or `loadSession()` support, you can reconnect to the same conversation with:

```bash
npx fireline repl <session-id>
```

If your host is on a different port:

```bash
FIRELINE_URL=http://127.0.0.1:4450 npx fireline repl <session-id>
```

Two practical rules matter here:

- save the session id the REPL header shows
- do not expect reconnect after exiting `fireline run ... --repl`, because that
  command tears the local host down as part of shutdown

On current `main`, the CLI reconnect path works like this:

- if the agent advertises `resume`, Fireline uses that
- otherwise, if the agent advertises `loadSession`, Fireline uses that
- otherwise, the CLI fails fast and tells you the agent does not support
  reattachment

## 7. When To Reach For `examples/approval-workflow`

The frozen demo asset is the right first run because it stays small and easy to
explain. Move to
[`examples/approval-workflow/`](../../../examples/approval-workflow/README.md)
when you need the fuller operator shape:

- `approve(...)` plus an external approval resolver
- `secretsProxy(...)` with `env:*` bindings
- `fireline.db(...)` subscriptions over `permissions`
- `handle.resolvePermission(...)` to continue the same run after a decision

That example is the current reference for "real local agent with approval and
secret wiring" on top of the same compose/start surface.

## Next Steps

- [Quickstart](./quickstart.md) for the smallest landed proof path
- [Compose and Start](../compose-and-start.md) for the full harness surface
- [Middleware](../middleware.md) for each middleware entry
- [Environment and Config](../api/env-and-config.md) for env vars and secret bindings
- [Crash-Proof Sessions](./crash-proof-sessions.md) for the kill-and-resume recipe
