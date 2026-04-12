# Compose and Start

## Current API surface

Today, the TypeScript control-plane API is:

- `compose(sandbox(...), middleware([...]), agent([...]))`
- `.as('name')`
- `.start({ serverUrl, name?, stateStream?, token? })`

See:

- [packages/client/src/sandbox.ts](../../packages/client/src/sandbox.ts)
- [packages/client/src/types.ts](../../packages/client/src/types.ts)

The client does **not** currently implement:

- local no-URL `start()`
- `start({ remote: '...' })`
- a live `FirelineAgent` wrapper object

Those are proposal-level ideas, not the current package surface.

## Define a harness

```ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

const reviewer = compose(
  sandbox({
    provider: 'docker',
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

`compose(...)` returns a `Harness`. It is still just a serializable spec at this point.

## Start a harness

```ts
const handle = await reviewer.start({
  serverUrl: 'http://127.0.0.1:4440',
  name: 'reviewer',
  stateStream: 'demo-reviewer',
})

console.log(handle.id)
console.log(handle.acp.url)
console.log(handle.state.url)
```

Internally, `start()` creates a `Sandbox` client and POSTs a provision request to `/v1/sandboxes`.

## What you get back

`start()` returns a `HarnessHandle`, not a live session wrapper. The handle contains:

- `id` for lifecycle operations
- `provider` for debugging or routing
- `acp.url` for ACP
- `state.url` for `@fireline/state`
- `name` for topology bookkeeping

If you want a single object with `connect()`, `resolvePermission()`, `stop()`, and `destroy()`, that convenience layer does not exist yet.

Today you compose it yourself:

- ACP connection: `@agentclientprotocol/sdk` or `use-acp`
- observation: `createFirelineDB({ stateStreamUrl: handle.state.url })`
- destroy: `new SandboxAdmin({ serverUrl }).destroy(handle.id)`
- external approval resolution: `appendApprovalResolved(...)`

## Connect to ACP

In React, the cleanest path is `use-acp`:

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

In Node, use `@agentclientprotocol/sdk` directly. The repo’s example helper in [examples/shared/acp-node.ts](../../examples/shared/acp-node.ts) is a compact reference implementation.

## Observe the state stream

```ts
import { createFirelineDB } from '@fireline/state'

const db = createFirelineDB({ stateStreamUrl: handle.state.url })
await db.preload()

db.collections.sessions.subscribe((rows) => {
  console.log(rows.map((row) => row.sessionId))
})
```

## Destroy a sandbox

```ts
import { SandboxAdmin } from '@fireline/client/admin'

const admin = new SandboxAdmin({ serverUrl: 'http://127.0.0.1:4440' })
await admin.destroy(handle.id)
```

There is no root export for `SandboxAdmin`; import it from `@fireline/client/admin`.

## Multi-agent topologies

The current client exports three topology helpers from [packages/client/src/topology.ts](../../packages/client/src/topology.ts):

- `peer(...)`
- `fanout(...)`
- `pipe(...)`

Example:

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

const replicas = await fanout(worker('reviewer'), { count: 3 }).start({
  serverUrl: 'http://127.0.0.1:4440',
  name: 'reviewer',
})

const pipeline = await pipe(worker('researcher'), worker('writer')).start({
  serverUrl: 'http://127.0.0.1:4440',
  stateStream: 'pipeline-demo',
})
```

Important current behavior:

- topology-level `peer(...)` shares a state stream and starts harnesses in parallel
- it does **not** inject the `peer_mcp` component by itself
- if you want cross-agent MCP routing, add middleware-level `peer()` inside each harness
- `pipe(...)` starts sequentially with a shared state stream, but it does **not** automatically pipe one agent’s output into the next
