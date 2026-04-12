# Compose and Start

## Current API surface

The TypeScript control-plane API is:

- `compose(sandbox(...), middleware([...]), agent([...]))`
- `.as('name')`
- `.start({ serverUrl, name?, stateStream?, token? })` → `Promise<FirelineAgent>`

See:

- [packages/client/src/sandbox.ts](../../packages/client/src/sandbox.ts)
- [packages/client/src/agent.ts](../../packages/client/src/agent.ts)
- [packages/client/src/types.ts](../../packages/client/src/types.ts)

## Define a harness

```ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

const reviewer = compose(
  sandbox({
    provider: 'docker',
    image: 'node:22-slim',
    resources: [localPath(process.cwd(), '/workspace', true)],
    labels: { team: 'infra', role: 'reviewer' },
  }),
  middleware([
    trace({ includeMethods: ['session/new', 'session/prompt'] }),
    approve({ scope: 'tool_calls' }),
  ]),
  agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
).as('reviewer')
```

`compose(...)` returns a `Harness`. It is still a serializable spec at
this point — no sandbox has been provisioned.

## Start a harness

```ts
const agent = await reviewer.start({
  serverUrl: 'http://127.0.0.1:4440',
  name: 'reviewer',
  stateStream: 'demo-reviewer',
})

console.log(agent.id)
console.log(agent.acp.url)
console.log(agent.state.url)
```

`start()` returns a **`FirelineAgent`** — a live object that preserves the
sandbox-handle fields and adds imperative methods:

| Field / method | Purpose |
|---|---|
| `agent.id` | Provider-assigned sandbox identifier |
| `agent.provider` | Provider name that created the sandbox |
| `agent.acp` | `{ url, headers? }` ACP endpoint |
| `agent.state` | `{ url, headers? }` durable state endpoint |
| `agent.name` | Logical harness name |
| `agent.connect(clientName?)` | Open an ACP WebSocket connection |
| `agent.resolvePermission(sessionId, requestId, outcome)` | Append `approval_resolved` to the state stream |
| `agent.stop()` / `agent.destroy()` | Tear down the sandbox via the control plane |

`FirelineAgent` implements `HarnessHandle`, so anything that expected the
bare handle fields still works.

### Current requirement: `serverUrl`

`start()` currently requires a `serverUrl` pointing at a running Fireline
control plane. Two ways to run one:

1. **Manual (dev):** `cargo build --bin fireline --bin fireline-streams`,
   then run `fireline-streams` and `fireline --control-plane --port 4440
   --durable-streams-url http://127.0.0.1:7474/v1/stream`.
2. **CLI (recommended):** `npx fireline run agent.ts` — the CLI spawns
   both binaries, provisions the sandbox, and prints the endpoints. See
   [cli.md](./cli.md).

The proposal vocabulary sometimes talks about `start()` with no arguments
(local embedded mode) or `start({ remote: '...' })`. Those are on the
[Declarative Agent API design](../proposals/declarative-agent-api-design.md)
roadmap; they are not in the package today. `serverUrl` is still the only
way to tell the client where to provision.

Upcoming identity note: the target agent-plane identifier cleanup is
tracked in [ACP Canonical Identifiers](../proposals/acp-canonical-identifiers.md).
That proposal is not the live runtime yet; it is the design reference
for the upcoming canonical `SessionId` / `RequestId` / `ToolCallId`
migration.

## Connect to ACP

```ts
const acp = await agent.connect('reviewer-ui')
const { sessionId } = await acp.newSession({ cwd: '/workspace', mcpServers: [] })
await acp.prompt({
  sessionId,
  prompt: [{ type: 'text', text: 'Review this repo.' }],
})
await acp.close()
```

`agent.connect()` returns a `ConnectedAcp` — a `ClientSideConnection` from
`@agentclientprotocol/sdk` with a `.close()` method added. If you prefer
the standalone helper, import `connectAcp` directly:

```ts
import { connectAcp } from '@fireline/client'

const acp = await connectAcp(agent.acp, 'reviewer-ui')
```

In React, use `use-acp` instead:

```tsx
import { useAcpClient } from 'use-acp'

function SessionView({ acpUrl }: { acpUrl: string }) {
  const acp = useAcpClient({
    wsUrl: acpUrl,
    autoConnect: true,
    sessionParams: { cwd: '/workspace', mcpServers: [] },
  })
  return <pre>{acp.connectionState.status}</pre>
}
```

## Observe the state stream

```ts
import fireline from '@fireline/client'

const db = await fireline.db({ stateStreamUrl: agent.state.url })

db.sessions.subscribe((rows) => {
  console.log(rows.map((row) => row.sessionId))
})
```

See [observation.md](./observation.md) for the full surface.

## Stop a sandbox

```ts
await agent.stop()
```

That calls the control plane's `DELETE /v1/sandboxes/:id`. If you don't
have the `FirelineAgent` object — for example, because a different process
provisioned it — use `SandboxAdmin`:

```ts
import { SandboxAdmin } from '@fireline/client/admin'

const admin = new SandboxAdmin({ serverUrl: 'http://127.0.0.1:4440' })
await admin.destroy(sandboxId)
```

There is no root export for `SandboxAdmin`; import it from
`@fireline/client/admin`.

## Multi-agent topologies

The client exports three topology helpers from
[packages/client/src/topology.ts](../../packages/client/src/topology.ts):

- `peer(...)`
- `fanout(...)`
- `pipe(...)`

```ts
import { agent, compose, fanout, middleware, peer as startPeers, pipe, sandbox } from '@fireline/client'
import { peer as peerMiddleware, trace } from '@fireline/client/middleware'

const worker = (name: string) =>
  compose(
    sandbox(),
    middleware([trace(), peerMiddleware()]),
    agent(['../../target/debug/fireline-testy']),
  ).as(name)

const peers = await startPeers(worker('agent-a'), worker('agent-b')).start({
  serverUrl: 'http://127.0.0.1:4440',
  stateStream: 'team-demo',
})
// → { 'agent-a': FirelineAgent, 'agent-b': FirelineAgent }

const replicas = await fanout(worker('reviewer'), { count: 3 }).start({
  serverUrl: 'http://127.0.0.1:4440',
  name: 'reviewer',
})

const pipeline = await pipe(worker('researcher'), worker('writer')).start({
  serverUrl: 'http://127.0.0.1:4440',
  stateStream: 'pipeline-demo',
})
```

Current behavior:

- topology-level `peer(...)` shares a state stream and starts harnesses in
  parallel; it does **not** inject the `peer_mcp` middleware component by
  itself. Add middleware-level `peer({ peers: [...] })` inside each
  harness for cross-agent MCP routing.
- `peer({ peers: [...] })` now forwards the peer list to the topology
  component (this was a silent drop before — fixed).
- `pipe(...)` starts sequentially with a shared state stream; it does
  **not** automatically pipe output between agents.
