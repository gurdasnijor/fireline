# 5-minute Quickstart

You want one proof point fast: Fireline can boot an ACP agent, print stable
ACP and state endpoints, and let you read the response back from the durable
stream.

This guide uses the smallest landed path that works cleanly on current `main`:
a tiny `claude-acp` spec you run with `npx fireline`. Once that works, compare
it to the fuller frozen demo harness in
[`docs/demos/assets/agent.ts`](../../demos/assets/agent.ts).

## What you'll do

- install the workspace dependencies and local binaries once
- export a minimal `compose(...)` spec
- run `npx fireline quickstart-agent.ts`
- send one prompt over ACP and read the reply from the state stream

## Prerequisites

- Node `>=20`
- `pnpm`
- a working Rust toolchain
- `ANTHROPIC_API_KEY` in your shell
- internet access on the first run so `claude-acp` can be resolved if it is not
  already cached locally

## 1. Build the local CLI once

From the repo root:

```bash
pnpm install
pnpm --filter @fireline/cli build
cargo build --bin fireline --bin fireline-streams

export FIRELINE_BIN="$PWD/target/debug/fireline"
export FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams"
export ANTHROPIC_API_KEY="..."
```

Why these steps exist:

- `packages/fireline/bin/fireline.js` loads `packages/fireline/dist/cli.js`, so
  the CLI package needs one TypeScript build first.
- `npx fireline` then resolves the local `fireline` and `fireline-streams`
  binaries from `target/debug` or `target/release`, matching the current CLI
  behavior in [`packages/fireline/src/cli.ts`](../../packages/fireline/src/cli.ts).

## 2. Export a minimal spec

Create `quickstart-agent.ts` in the repo root:

```bash
cat > quickstart-agent.ts <<'EOF'
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'

export default compose(
  sandbox({}),
  middleware([trace()]),
  agent(['claude-acp']),
)
EOF
```

This is intentionally smaller than the frozen demo asset. It proves the basic
loop first: boot Fireline, connect over ACP, send a prompt, observe the answer.

## 3. Start Fireline

In terminal 1:

```bash
npx fireline quickstart-agent.ts
```

If `:4440` or `:7474` is already in use, rerun with explicit ports:

```bash
npx fireline quickstart-agent.ts --port 15443 --streams-port 17477
```

Expected boot output:

```text
durable-streams ready at http://127.0.0.1:17477/v1/stream

  ✓ fireline ready

    sandbox:   runtime:6f07cf50-560b-4c04-b037-acc954132d73
    ACP:       ws://127.0.0.1:52403/acp
    state:     http://127.0.0.1:17477/v1/stream/fireline-state-runtime-6f07cf50-560b-4c04-b037-acc954132d73

  Press Ctrl+C to shut down.
```

Keep this terminal open. Copy the printed `ACP` and `state` URLs.

## 4. Send one prompt and print the streamed reply

In terminal 2, set the URLs you just copied:

```bash
export ACP_URL='ws://127.0.0.1:52403/acp'
export STATE_URL='http://127.0.0.1:17477/v1/stream/fireline-state-runtime-6f07cf50-560b-4c04-b037-acc954132d73'
```

Then send one deterministic prompt and read the text chunks back from the state
stream:

```bash
node --input-type=module <<'EOF'
import fireline, { connectAcp } from '@fireline/client'

const acp = await connectAcp(process.env.ACP_URL, 'quickstart-cli')
const db = await fireline.db({ stateStreamUrl: process.env.STATE_URL })

const { sessionId } = await acp.newSession({ cwd: process.cwd(), mcpServers: [] })
const response = await acp.prompt({
  sessionId,
  prompt: [{ type: 'text', text: 'Reply with exactly: hello from Fireline' }],
})

const deadline = Date.now() + 30_000
let text = ''
while (Date.now() < deadline) {
  text = db.chunks.toArray
    .filter((row) =>
      row.sessionId === sessionId &&
      row.update.sessionUpdate === 'agent_message_chunk' &&
      typeof row.update.content?.text === 'string',
    )
    .map((row) => row.update.content.text)
    .join('')
  if (text.length > 0) break
  await new Promise((resolve) => setTimeout(resolve, 100))
}

console.log(JSON.stringify({ stopReason: response.stopReason, text }, null, 2))

await acp.close()
db.close()
EOF
```

Success looks like this:

```json
{
  "stopReason": "end_turn",
  "text": "hello from Fireline"
}
```

You may also see `StreamDB` preload logs before the final JSON block. The real
success signal is the final `stopReason` plus the streamed text.

## 5. What just happened

- `npx fireline quickstart-agent.ts` started a local control plane, started
  `durable-streams`, provisioned the spec, and printed the two endpoints you use
  next.
- `ACP` is the session plane. `connectAcp(...)` uses it for `session/new` and
  `session/prompt`.
- `state` is the observation plane. `fireline.db(...)` tails the durable stream
  and lets you read the returned chunks reactively.
- `agent(['claude-acp'])` is the minimal landed registry-backed agent path.

## Why this starts smaller than `docs/demos/assets/agent.ts`

The frozen demo harness in [`docs/demos/assets/agent.ts`](../../demos/assets/agent.ts)
is the right file to inspect next because it shows the full middleware stack:

- `trace()`
- `approve({ scope: 'tool_calls' })`
- `budget({ tokens: 2_000_000 })`
- `secretsProxy(...)`
- `peer({ peers: ['reviewer'] })`

That fuller recipe is documented in
[`docs/demos/assets/README.md`](../../demos/assets/README.md). The Quickstart
starts smaller so your first run proves the substrate before you add approvals,
budgets, secret injection, and peer routing.

## Next steps

- Read [CLI](../cli.md) for the full `run`, `build`, `deploy`, and `agents`
  surface.
- Read [Concepts](../concepts.md) for the three-plane model.
- Read [Middleware](../middleware.md) before you add `approve()`, `budget()`, or
  `secretsProxy()`.
- Read [Approvals](../approvals.md) when you want durable human gates around tool
  calls.
- Read [Observation](../observation.md) when you are ready to turn the state stream into a dashboard or live query surface.
