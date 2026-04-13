# Multi-agent

Fireline can run more than one agent in the same workflow. The two core ideas are:

- topology helpers decide how many agents you start
- middleware decides what those agents can do once they are running

If you want one agent to discover and call another agent, you need both:

- a shared discovery surface
- `peer()` middleware inside the participating specs

## What This Does

Use Fireline multi-agent mode when one agent should hand work to another agent without leaving the Fireline runtime. Common examples:

- a planner asks a reviewer to check a draft
- a support agent hands a billing task to a finance agent
- one agent fans work out to a small pool of identical workers

What you get on the shipped surface today:

- one Fireline control plane per host
- shared durable-stream discovery when agents need to find each other
- built-in peer tools: `list_peers` and `prompt_peer`
- child-session lineage in the state stream
- W3C trace context carried across a peer hop through `_meta.traceparent`, `_meta.tracestate`, and `baggage`

## Pick The Right Primitive

Fireline has two different `peer` concepts, and they are easy to confuse:

- `peer(...)` from `@fireline/client`
  Starts multiple named agents against one shared state stream.
- `peer()` from `@fireline/client/middleware`
  Enables the `fireline-peer` MCP tools that let an agent discover and call another agent.

The other topology helpers are still useful:

- `pipe(...)`
  Start a fixed set of agents on one shared stream when your app will orchestrate the handoff itself.
- `fanout(...)`
  Start `N` copies of the same spec.

Important boundary:

- topology helpers are provisioning helpers, not a workflow engine
- `pipe(...)` does not automatically pass output from agent A into agent B
- `peer(...)` does not automatically enable peer discovery unless the harness also includes middleware `peer()`

## Fastest Way To See It Working

The quickest end-to-end example on `main` is the cross-host discovery example:

```bash
cargo build --bin fireline --bin fireline-streams --bin fireline-testy
cd examples/cross-host-discovery
pnpm install
pnpm run dev
```

Expected output excerpt:

```json
{
  "serverA": "http://127.0.0.1:4440",
  "serverB": "http://127.0.0.1:5440",
  "agentA": "ws://127.0.0.1:.../acp",
  "agentB": "ws://127.0.0.1:.../acp",
  "peers": "...agent-a...agent-b...",
  "promptPeer": "...hello across hosts..."
}
```

What that example proves:

- two control planes can publish into the same durable-streams deployment
- `list_peers` sees both agents
- `prompt_peer` opens a child session on the remote agent and returns the result to the caller

If you want the same shape with a replayable operator script, use:

- [docs/demos/peer-to-peer-demo-capture.md](../demos/peer-to-peer-demo-capture.md)
- [docs/demos/scripts/replay-peer-to-peer.sh](../demos/scripts/replay-peer-to-peer.sh)

## Compose Shape

This is the minimal pattern to remember:

```ts
import { agent, compose, middleware, peer as startPeers, sandbox } from '@fireline/client'
import { peer as peerMiddleware, trace } from '@fireline/client/middleware'

const stage = (name: string) =>
  compose(
    sandbox(),
    middleware([trace(), peerMiddleware({ peers: ['reviewer'] })]),
    agent(['../../target/debug/fireline-testy']),
  ).as(name)

const handles = await startPeers(
  stage('planner'),
  stage('reviewer'),
).start({
  serverUrl: 'http://127.0.0.1:4440',
  stateStream: 'multi-agent-demo',
})
```

That start call does two things:

- provisions both agents
- places them on the same state stream

After that, the caller can use the built-in peer MCP tools:

```ts
const acp = await handles.planner.connect('multi-agent-guide')
const { sessionId } = await acp.newSession({ cwd: process.cwd(), mcpServers: [] })

const toolCall = (tool: string, params: Record<string, unknown> = {}) =>
  JSON.stringify({ command: 'call_tool', server: 'fireline-peer', tool, params })

await acp.prompt({
  sessionId,
  prompt: [{ type: 'text', text: toolCall('list_peers') }],
})

await acp.prompt({
  sessionId,
  prompt: [{
    type: 'text',
    text: toolCall('prompt_peer', {
      agentName: 'reviewer',
      prompt: JSON.stringify({ command: 'echo', message: 'please review this' }),
    }),
  }],
})
```

Expected behavior:

- `list_peers` returns both `planner` and `reviewer`
- `prompt_peer` returns the remote agent name plus the remote response text
- the remote side gets a fresh child session, not a reused parent session

## What You Should See In The Stream

When a peer hop succeeds, the state stream tells a clear story:

- the caller records a prompt for the `prompt_peer` tool call
- the caller records a `child_session_edge`
- the callee records a new child session and prompt request
- the callee emits its own chunks
- the caller receives the remote result back in its own session

In practice, this means:

- use the caller stream to understand who delegated
- use the callee stream to understand what the child agent actually did
- use trace context to follow the hop in your observability backend

## What Could Go Wrong

- Agents on different durable-streams deployments will not discover each other.
  If `agent-a` and `agent-b` boot against different discovery streams, `list_peers` will only see the local runtime and `prompt_peer` will fail with `peer '<name>' not found`.
- Topology `peer(...)` does not replace middleware `peer()`.
  Starting two agents on the same stream is not enough; the calling agent still needs the built-in peer tools enabled.
- A peer hop creates a child session.
  This is the right shape, but it means you should expect a new session id and a new request id on the callee side.
- Older demo artifacts may describe the pre-`429475e` lineage gap.
  Current `main` carries W3C trace context across the peer hop, but older FQA notes and captures were recorded before that fix landed.

## When To Reach For `pipe(...)` Instead

Use `pipe(...)` when your application, not the agent, should control the handoff. A good example is:

- [examples/multi-agent-team/index.ts](../../examples/multi-agent-team/index.ts)

That example starts two agents on one shared stream, waits for the researcher to finish, then sends the writer a second prompt from the host application. This is often the simpler fit when:

- the host app already owns the workflow state
- you want deterministic sequencing
- you do not need one agent to discover another agent dynamically

## Deeper References

- [examples/cross-host-discovery/README.md](../../examples/cross-host-discovery/README.md)
- [docs/demos/peer-to-peer-demo-capture.md](../demos/peer-to-peer-demo-capture.md)
- [docs/proposals/deployment-and-remote-handoff.md](../proposals/deployment-and-remote-handoff.md) for the spec-level design direction
- [docs/proposals/durable-subscriber.md](../proposals/durable-subscriber.md) for the substrate-level trace and delivery model
