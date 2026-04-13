# Your First Agent

You want the smallest Fireline program you can own directly in TypeScript.

This recipe is the package-API complement to
[Quickstart](./quickstart.md):

- Quickstart uses `npx fireline` and a registry-backed agent path.
- This guide uses `@fireline/client` directly and a local deterministic test
  agent.

That makes the first run smaller, local, and repeatable.

## What You'll Build

By the end of this recipe you will have a script that:

- composes a harness with `sandbox(...)`, `middleware(...)`, and `agent(...)`
- provisions it with `.start({ serverUrl })`
- opens ACP with `handle.connect(...)`
- sends one prompt
- reads the reply from the durable state stream with `fireline.db(...)`

The agent binary in this guide is `fireline-testy-prompt`, which simply echoes
the prompt text back as an `agent_message_chunk`. No model key or external
service is required.

## Prerequisites

- Node `>=20`
- `pnpm`
- a working Rust toolchain
- the local Fireline binaries built once:
  - `fireline`
  - `fireline-streams`
  - `fireline-testy-prompt`

From the repo root:

```bash
pnpm install
cargo build --bin fireline --bin fireline-streams --bin fireline-testy-prompt
```

## 1. Start Durable Streams

In terminal 1:

```bash
./target/debug/fireline-streams
```

## 2. Start One Fireline Host

In terminal 2:

```bash
./target/debug/fireline --control-plane --port 4440 \
  --durable-streams-url http://127.0.0.1:7474/v1/stream
```

This guide uses the manual host path so you can see what the package API talks
to. If you want the CLI-managed path instead, use [Quickstart](./quickstart.md).

## 3. Create `first-agent.ts`

Create `first-agent.ts` in the repo root:

```bash
cat > first-agent.ts <<'EOF'
import { resolve } from 'node:path'
import fireline, { agent, compose, middleware, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'
import { extractChunkTextPreview } from '@fireline/state'

const serverUrl = process.env.FIRELINE_URL ?? 'http://127.0.0.1:4440'
const agentBin =
  process.env.AGENT_BIN ?? resolve(process.cwd(), 'target/debug/fireline-testy-prompt')

const handle = await compose(
  sandbox({
    provider: 'local',
    labels: { demo: 'first-agent' },
  }),
  middleware([
    trace({ includeMethods: ['session/new', 'session/prompt'] }),
  ]),
  agent([agentBin]),
).start({
  serverUrl,
  name: 'first-agent',
})

const db = await fireline.db({ stateStreamUrl: handle.state.url })
const acp = await handle.connect('first-agent')

const { sessionId } = await acp.newSession({
  cwd: process.cwd(),
  mcpServers: [],
})

const promptText = 'hello from Fireline'
const response = await acp.prompt({
  sessionId,
  prompt: [{ type: 'text', text: promptText }],
})

const echoed = await new Promise<string>((resolveText, reject) => {
  let subscription: { unsubscribe(): void }
  const timeout = setTimeout(() => {
    subscription.unsubscribe()
    reject(new Error('timed out waiting for response chunks'))
  }, 5_000)

  subscription = db.chunks.subscribe((rows) => {
    const text = rows
      .filter((row) => row.sessionId === sessionId)
      .map((row) => extractChunkTextPreview(row.update))
      .join('')

    if (!text) return

    clearTimeout(timeout)
    subscription.unsubscribe()
    resolveText(text)
  })
})

console.log(
  JSON.stringify(
    {
      sandboxId: handle.id,
      sessionId,
      stopReason: response.stopReason,
      text: echoed,
      stateStreamUrl: handle.state.url,
    },
    null,
    2,
  ),
)

await acp.close()
db.close()
await handle.stop()
EOF
```

Why this shape:

- `sandbox({ provider: 'local' })` keeps the first run local
- `trace(...)` gives the harness a durable audit stream
- `agent([agentBin])` points at a deterministic local ACP agent
- `fireline.db(...)` turns the state stream into a live read model

## 4. Run It

From the repo root:

```bash
pnpm exec tsx first-agent.ts
```

If you want to override the host or agent binary path:

```bash
FIRELINE_URL=http://127.0.0.1:4440 \
AGENT_BIN="$PWD/target/debug/fireline-testy-prompt" \
pnpm exec tsx first-agent.ts
```

## 5. Check The Result

Success looks like this shape:

```json
{
  "sandboxId": "runtime:...",
  "sessionId": "session-...",
  "stopReason": "end_turn",
  "text": "hello from Fireline",
  "stateStreamUrl": "http://127.0.0.1:7474/v1/stream/..."
}
```

What that proves:

- the harness provisioned successfully
- ACP `session/new` and `session/prompt` both worked
- the reply was visible through the durable state stream
- the public package surface is enough to run the full loop without the CLI

## 6. What Just Happened

The control flow is:

1. `compose(...)` builds a serializable harness spec
2. `.start({ serverUrl })` provisions a sandbox on the Fireline host
3. `handle.connect(...)` opens an ACP connection to the sandbox's ACP endpoint
4. `fireline.db({ stateStreamUrl: handle.state.url })` tails the durable state
   stream
5. `db.chunks.subscribe(...)` reacts when the echoed response chunk arrives

The three planes are already visible even in this tiny example:

- control plane: `.start({ serverUrl })`
- session plane: `handle.connect(...)`, `newSession(...)`, `prompt(...)`
- observation plane: `fireline.db(...)`

## Why This Uses `fireline-testy-prompt`

`fireline-testy-prompt` is the simplest deterministic ACP agent in this repo.
Its behavior on current `main` is:

- `newSession(...)` returns a fresh `SessionId`
- `prompt(...)` concatenates text blocks from the request
- it emits that text as an `agent_message_chunk`
- it returns `stopReason: 'end_turn'`

That makes it a good first-agent target because the result is local and
predictable.

## Where To Go Next

- [Quickstart](./quickstart.md) for the CLI-managed path
- [Compose and Start](../compose-and-start.md) for the full `Harness` and
  `FirelineAgent` surface
- [Observation](../observation.md) for the durable read-model pattern
- [Environment and Config](../api/env-and-config.md) for host URLs, sandbox
  config, and env overrides
- [`@fireline/state`](../api/state.md) for the observation package reference
