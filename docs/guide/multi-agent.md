# Multi-agent Topologies

The TypeScript topology helpers live in [packages/client/src/topology.ts](../../packages/client/src/topology.ts):

- `peer(...)`
- `fanout(...)`
- `pipe(...)`

These are real exports from `@fireline/client` today.

## `peer(...)`

```ts
import { agent, compose, middleware, peer as startPeers, sandbox } from '@fireline/client'
import { peer as peerMiddleware, trace } from '@fireline/client/middleware'

const stage = (name: string) =>
  compose(
    sandbox({ labels: { role: name } }),
    middleware([trace(), peerMiddleware()]),
    agent(['../../target/debug/fireline-testy']),
  ).as(name)

const handles = await startPeers(stage('agent-a'), stage('agent-b')).start({
  serverUrl: 'http://127.0.0.1:4440',
  stateStream: 'peer-demo',
})
```

What it actually does today:

- starts all harnesses in parallel
- shares one `stateStream` across them
- returns a name-keyed object of `HarnessHandle`s

What it does **not** do by itself:

- it does not inject `peer_mcp`
- it does not automatically create cross-agent routing

If you want peer discovery and `prompt_peer`, add middleware-level `peer()` inside each harness.

## `fanout(...)`

```ts
import { agent, compose, fanout, middleware, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'

const reviewer = compose(
  sandbox(),
  middleware([trace()]),
  agent(['../../target/debug/fireline-testy']),
).as('reviewer')

const handles = await fanout(reviewer, { count: 3 }).start({
  serverUrl: 'http://127.0.0.1:4440',
  name: 'reviewer',
})
```

Actual behavior:

- starts `count` copies in parallel
- suffixes names as `reviewer-1`, `reviewer-2`, `reviewer-3`
- returns an array of handles

`fanout(...)` is just provisioning topology. It does not merge outputs or schedule work across replicas for you.

## `pipe(...)`

```ts
import { agent, compose, middleware, pipe, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'

const stage = (name: string) =>
  compose(sandbox(), middleware([trace()]), agent(['../../target/debug/fireline-testy'])).as(name)

const handles = await pipe(stage('researcher'), stage('writer')).start({
  serverUrl: 'http://127.0.0.1:4440',
  stateStream: 'pipeline-demo',
})
```

Actual behavior:

- starts harnesses sequentially
- shares one state stream
- returns a name-keyed object of handles

Important limitation:

- `pipe(...)` does **not** automatically pass agent A’s output into agent B’s input
- you still orchestrate the ACP session flow yourself

See the current example:

- [examples/multi-agent-team/index.ts](../../examples/multi-agent-team/index.ts)

That example starts a pipeline topology, then manually connects to both ACP endpoints and sends the second agent the first agent’s output.

## Topology helpers are provisioning helpers

This is the easiest way to reason about the current implementation:

- topology helpers decide how many harnesses to start
- they decide whether those harnesses share a state stream
- they return handles

They do **not** yet form a high-level workflow runtime on their own.

The actual multi-agent coordination still happens in:

- ACP prompts
- shared observation through the durable stream
- optional middleware such as `peer()`
