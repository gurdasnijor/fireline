# `@fireline/client`

Calling an ACP agent directly gets you a session. It does not give you a
portable harness, a durable state stream, approval resolution, or a clean way
to run the same agent locally and remotely.

`@fireline/client` is the TypeScript package for that missing layer. You use it
to:

- declare a Fireline harness with `sandbox(...)`, `middleware(...)`, and `agent(...)`
- start that harness on a Fireline host with `compose(...).start(...)`
- connect to the agent over ACP
- observe the run through the durable state stream
- start small multi-agent topologies with `peer(...)`, `fanout(...)`, and `pipe(...)`

## Package Map

- `@fireline/client`
  core harness, runtime, topology, ACP, and DB helpers
- `@fireline/client/middleware`
  middleware builders such as `trace()`, `approve()`, `budget()`, and `secretsProxy()`
- `@fireline/client/resources`
  mount helpers such as `localPath(...)`
- `@fireline/client/admin`
  sandbox admin reads and deletes
- `@fireline/client/workflow`
  awakeable and completion-key helpers

## Fastest Path

```ts
import fireline, { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, budget, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

const reviewer = compose(
  sandbox({
    provider: 'local',
    resources: [localPath('.', '/workspace')],
    labels: { role: 'reviewer' },
  }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    budget({ tokens: 500_000 }),
  ]),
  agent(['npx', '-y', '@agentclientprotocol/claude-agent-acp']),
).as('reviewer')

const handle = await reviewer.start({
  serverUrl: 'http://127.0.0.1:4440',
  stateStream: 'demo:reviewer',
})

const acp = await handle.connect('reviewer-ui')
const db = await fireline.db({ stateStreamUrl: handle.state.url })
```

That is the package's main story on current `main`: author a harness once, boot
it on a Fireline host, talk to it over ACP, and watch the durable stream update
in real time.

## Root Surface

### `compose(sandboxConfig, middlewareConfig, agentConfig)`

Builds one runnable harness value. This is the product surface you keep in
source control and reuse across local dev, demos, and hosted runs.

```ts
import { agent, compose, middleware, sandbox } from '@fireline/client'

const harness = compose(
  sandbox({ provider: 'local' }),
  middleware([]),
  agent(['../../target/debug/fireline-testy-load']),
)
```

### `sandbox(options?)`

Defines where the agent runs and what it can see. Put provider choice, labels,
resources, and env vars here.

```ts
import { sandbox } from '@fireline/client'
import { localPath } from '@fireline/client/resources'

const sandboxConfig = sandbox({
  provider: 'docker',
  image: 'node:22-slim',
  resources: [localPath('.', '/workspace')],
  labels: { demo: 'code-review' },
})
```

### `middleware(chain)`

Wraps an ordered middleware array into a serializable chain. Order matters: the
host lowers this array in order when it provisions the runtime.

```ts
import { middleware } from '@fireline/client'
import { approve, budget, trace } from '@fireline/client/middleware'

const chain = middleware([
  trace(),
  approve({ scope: 'tool_calls' }),
  budget({ tokens: 250_000 }),
])
```

### `agent(command)`

Defines the ACP-speaking process Fireline should launch inside the sandbox.

```ts
import { agent } from '@fireline/client'

const agentConfig = agent(['npx', '-y', '@agentclientprotocol/claude-agent-acp'])
```

Single-token ACP registry ids such as `agent(['pi-acp'])` are also valid on
current `main`.

### `harness.as(name)`

Gives the harness a stable logical name. Use this when you want readable handle
names or when you are composing topologies.

```ts
const reviewer = compose(
  sandbox({ provider: 'local' }),
  middleware([]),
  agent(['../../target/debug/fireline-testy-load']),
).as('reviewer')
```

### `harness.start(options)`

Provisions the harness on a Fireline host and returns a live `FirelineAgent`
handle with ACP and state endpoints plus runtime methods.

```ts
const handle = await reviewer.start({
  serverUrl: 'http://127.0.0.1:4440',
  stateStream: 'demo:reviewer',
})
```

`start(...)` is the API most apps should use. It is the landed replacement for
"provision a sandbox, then manually stitch the rest together."

### `handle.connect(clientName?)`

Opens a ready-to-use ACP client against the running agent.

```ts
const acp = await handle.connect('reviewer-ui')
const { sessionId } = await acp.newSession({ cwd: '/workspace', mcpServers: [] })
```

### `handle.resolvePermission(sessionId, requestId, outcome)`

Resolves a pending approval request back into the same durable run.

```ts
await handle.resolvePermission(sessionId, requestId, {
  allow: true,
  resolvedBy: 'reviewer-ui',
})
```

Use this when your app is the human-in-the-loop surface for
`approve({ scope: 'tool_calls' })`.

### `handle.stop()` / `handle.destroy()`

Stops the running sandbox for this handle.

```ts
await handle.stop()
```

`destroy()` is the same runtime operation when you prefer that verb:

```ts
await handle.destroy()
```

### `connectAcp(endpoint, clientName?)`

Connects directly when you already have an ACP endpoint and do not need a
`FirelineAgent` handle in hand.

```ts
import { connectAcp } from '@fireline/client'

const acp = await connectAcp('ws://127.0.0.1:4440/acp', 'dashboard')
```

Most app code should prefer `handle.connect(...)`, but `connectAcp(...)` is the
right lower-level helper when you are reconnecting from a saved endpoint.

### `fireline.db(options?)`

Opens the durable state DB and hoists the live collections directly onto the DB
instance.

```ts
import fireline from '@fireline/client'

const db = await fireline.db({ stateStreamUrl: handle.state.url })
const subscription = db.permissions.subscribe((rows) => {
  console.log('pending approvals', rows.filter((row) => row.state === 'pending').length)
})
```

The collection surface itself comes from `@fireline/state`. Reach for that
package when you want the collection and type reference directly.

### `new Sandbox({ serverUrl, token? })`

This is the lower-level client when you already have a serialized harness config
and want to provision it yourself.

```ts
import { Sandbox, agent, compose, middleware, sandbox } from '@fireline/client'

const client = new Sandbox({ serverUrl: 'http://127.0.0.1:4440' })
const handle = await client.provision(
  compose(
    sandbox({ provider: 'local' }),
    middleware([]),
    agent(['../../target/debug/fireline-testy-load']),
  ),
)
```

Most package users should prefer `compose(...).start(...)`. `Sandbox` is the
escape hatch when you need explicit provisioning control.

## Topology Helpers

These helpers start more than one harness for you. They all return objects with
their own `.start(...)` method.

### `peer(...harnesses)`

Starts named harnesses together on one shared `stateStream` and returns a handle
map keyed by harness name.

```ts
import { agent, compose, middleware, peer, sandbox } from '@fireline/client'
import { peer as peerMiddleware, trace } from '@fireline/client/middleware'

const reviewer = compose(
  sandbox({ provider: 'local' }),
  middleware([trace(), peerMiddleware({ peers: ['writer'] })]),
  agent(['../../target/debug/fireline-testy-load']),
).as('reviewer')

const writer = compose(
  sandbox({ provider: 'local' }),
  middleware([trace(), peerMiddleware({ peers: ['reviewer'] })]),
  agent(['../../target/debug/fireline-testy-load']),
).as('writer')

const handles = await peer(reviewer, writer).start({
  serverUrl: 'http://127.0.0.1:4440',
  stateStream: 'demo:peer',
})
```

Important: topology-level `peer(...)` starts the named harnesses together.
Middleware-level `peer({ peers: [...] })` is what turns on peer routing inside
each harness.

### `fanout(harness, { count })`

Starts `N` copies of the same harness for parallel work.

```ts
import { fanout } from '@fireline/client'

const workers = await fanout(
  compose(
    sandbox({ provider: 'local' }),
    middleware([]),
    agent(['../../target/debug/fireline-testy-load']),
  ).as('worker'),
  { count: 3 },
).start({
  serverUrl: 'http://127.0.0.1:4440',
  name: 'review-worker',
})
```

This returns an array of `FirelineAgent` handles with numbered runtime names
such as `review-worker-1`, `review-worker-2`, and `review-worker-3`.

### `pipe(...harnesses)`

Starts named harnesses sequentially on one shared `stateStream`.

```ts
import { pipe } from '@fireline/client'
import { trace } from '@fireline/client/middleware'

const researcher = compose(
  sandbox({ provider: 'local' }),
  middleware([trace()]),
  agent(['../../target/debug/fireline-testy-load']),
).as('researcher')

const writer = compose(
  sandbox({ provider: 'local' }),
  middleware([trace()]),
  agent(['../../target/debug/fireline-testy-load']),
).as('writer')

const handles = await pipe(researcher, writer).start({
  serverUrl: 'http://127.0.0.1:4440',
  stateStream: 'demo:pipe',
})
```

Use this when the stages should share one durable history but come up in a
deliberate order.

## Middleware Import Path

Middleware builders live on the subpath import:

```ts
import {
  attachTools,
  approve,
  autoApprove,
  budget,
  contextInjection,
  durableSubscriber,
  inject,
  peer as peerMiddleware,
  peerRouting,
  secretsProxy,
  telegram,
  trace,
  wakeDeployment,
  webhook,
} from '@fireline/client/middleware'
```

The most common starting point is still small:

```ts
import { middleware } from '@fireline/client'
import { approve, budget, secretsProxy, trace } from '@fireline/client/middleware'

const chain = middleware([
  trace(),
  approve({ scope: 'tool_calls' }),
  budget({ tokens: 500_000 }),
  secretsProxy({
    ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' },
  }),
])
```

## Types You Will Usually Reach For

The root package exports the runtime and authoring types most app code needs:

- `Harness`
- `HarnessHandle`
- `SandboxDefinition`
- `SandboxHandle`
- `StartOptions`
- `ConnectedAcp`
- `FirelineDB`
- `ResolvePermissionOutcome`
- `SessionId`
- `RequestId`
- `ToolCallId`

Example:

```ts
import type { FirelineDB, Harness, SessionId } from '@fireline/client'
```

## Related Docs

- [Compose and Start](../../docs/guide/compose-and-start.md)
- [Middleware](../../docs/guide/middleware.md)
- [Providers](../../docs/guide/providers.md)
- [Observation](../../docs/guide/observation.md)
- [`@fireline/state`](../state/README.md)
