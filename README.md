# Fireline

Open-source infrastructure for durable, composable AI agents.

```typescript
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, approve, budget } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

const handle = await compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([trace(), approve({ scope: 'tool_calls' }), budget({ tokens: 500_000 })]),
  agent(['claude-code-acp']),
).start({ serverUrl: 'http://localhost:4440' })
```

---

## What makes Fireline different

**Durable.** Sessions survive sandbox death. The [durable stream](https://durablestreams.com) is the source of truth, not any single process. Kill a sandbox, restart on a different host, run `session/load` — the conversation continues from exactly where it left off. Every ACP effect lands in an append-only log with idempotent writes and offset replay.

**Composable.** Middleware intercepts the ACP channel declaratively. `trace()`, `approve()`, `budget()`, `inject()` — each is a serializable spec, not a closure. The Rust conductor interprets them server-side. Compose them like Express middleware, ship them as data, validate them before deployment.

**Observable.** [`@fireline/state`](packages/state/) gives you reactive queries over the agent's durable stream. No polling. `useLiveQuery()` in React, `.subscribe()` in Node. The stream IS the observation API — sessions, turns, chunks, permissions, cross-agent lineage, all materialized by [TanStack DB](https://tanstack.com/db) with differential dataflow.

**Portable.** Same `compose()` call runs on a local subprocess, in a Docker container, on a [microsandbox](https://github.com/superradcompany/microsandbox) VM, or on a remote Fireline server. Swap the `serverUrl`, keep everything else. The agent code doesn't know or care where it's running.

---

## Quick start

Give an AI agent access to your codebase, with approval gates and a token budget — in 8 lines:

```typescript
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, approve, budget } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

const handle = await compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([trace(), approve({ scope: 'tool_calls' }), budget({ tokens: 500_000 })]),
  agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
).start({ serverUrl: 'http://localhost:4440' })

// handle.acp.url  → open an ACP session, start prompting
// handle.state.url → subscribe to live agent activity
```

The agent runs in an isolated sandbox with your workspace mounted read-only. Every tool call requires your approval. Every effect is logged to a durable stream you can replay, query, or pipe into a dashboard. Kill the sandbox, restart it on a different machine — the session continues from where it left off.

See [examples/](examples/) for more — background agents, Slackbots, multi-agent pipelines, session migration across hosts.

---

<picture>
  <img alt="The Three Planes" src="assets/three-planes.svg" width="100%">
</picture>

## The three planes

| Plane | Package | What it does |
|---|---|---|
| **Control** | `@fireline/client` | `compose(sandbox, middleware, agent).start()` — provision sandboxes, wire middleware, execute commands |
| **Session** | `@agentclientprotocol/sdk` | ACP over `handle.acp.url` — `newSession()`, `prompt()`, `loadSession()`. Third-party protocol; Fireline never wraps it. |
| **Observation** | `@fireline/state` | `createFirelineDB({ stateStreamUrl: handle.state.url })` → `useLiveQuery()` — reactive TanStack DB collections over the durable stream |

The control plane gives you a handle. The handle carries two endpoints. Each endpoint connects you to a different plane. No side channels.

---

<picture>
  <img alt="Middleware Pipeline" src="assets/middleware-pipeline.svg" width="100%">
</picture>

## Middleware

An ordered list of ACP interceptors. Each is a serializable spec — data, not a closure. The Rust conductor interprets them server-side.

```typescript
import { trace, approve, budget, inject } from '@fireline/client/middleware'

middleware([
  trace(),                              // Log every ACP effect to the durable stream
  approve({ scope: 'tool_calls' }),     // Require approval before tool execution
  budget({ tokens: 1_000_000 }),        // Hard token budget cap
  inject([                              // Prepend context to every prompt
    { kind: 'workspace_file', path: '/workspace/README.md' },
    { kind: 'datetime' },
  ]),
])
```

Middleware composes. Add `peer(['agent:reviewer'])` to route cross-agent calls. Add `secretsProxy({ OPENAI_KEY: { allow: 'api.openai.com' } })` to isolate credentials. The conductor processes them in order on every ACP message.

---

<picture>
  <img alt="Secrets Isolation" src="assets/secrets-isolation.svg" width="100%">
</picture>

## Secrets isolation — credentials the agent can't see

Agents need API keys to do useful work. But if the agent can see the key, it can exfiltrate it. Fireline's `secretsProxy()` middleware injects credentials at call time, scoped to specific domains, without ever exposing the plaintext to the agent.

```typescript
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, approve, secretsProxy } from '@fireline/client/middleware'

const handle = await compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    secretsProxy({
      GITHUB_TOKEN: { ref: 'secret:gh-pat', allow: 'api.github.com' },
      OPENAI_KEY:   { ref: 'secret:openai', allow: 'api.openai.com' },
    }),
  ]),
  agent(['claude-code-acp']),
).start({ serverUrl: 'http://localhost:4440' })
```

The agent sees tool schemas without credential parameters. The harness resolves `CredentialRef`s from your secret store (env vars in dev, vault in production), injects them into outbound requests for the allowed domain, and emits an audit envelope to the durable stream — all without the agent ever touching a plaintext token.

→ *Technical deep-dive:* [`docs/proposals/secrets-injection-component.md`](docs/proposals/secrets-injection-component.md)

---

## Observe everything — reactively, from the stream

Every agent effect lands on a durable stream. `@fireline/state` materializes it into reactive collections you query like a database — no polling, no custom APIs.

```typescript
import { createFirelineDB } from '@fireline/state'
import { useLiveQuery } from '@tanstack/react-db'
import { eq } from '@tanstack/db'

const db = createFirelineDB({ stateStreamUrl: handle.state.url })

// React component — re-renders automatically as the agent works
function AgentActivity({ sessionId }: { sessionId: string }) {
  const turns = useLiveQuery(q =>
    q.from({ t: db.collections.promptTurns })
      .where(({ t }) => eq(t.sessionId, sessionId))
  )
  const pending = useLiveQuery(q =>
    q.from({ p: db.collections.permissions })
      .where(({ p }) => eq(p.state, 'pending'))
  )
  return <>{turns.map(t => <Turn key={t.promptTurnId} turn={t} />)}</>
}
```

The stream contains sessions, turns, chunks, tool calls, permissions, cross-agent lineage — all queryable in real-time. Build a dashboard, wire up a Slack notification, trigger a webhook — the stream is the API.

---

## Durable orchestration — subscribe, don't poll

Need to know when an agent finishes? When it needs approval? When a multi-step pipeline advances? Subscribe to the stream. The durable log IS the orchestration mechanism.

```typescript
// Wait for agent completion — durably, without holding a connection open
db.collections.promptTurns.subscribe(turns => {
  const done = turns.find(t => t.sessionId === id && t.state === 'completed')
  if (done) notifyUser(done)
})

// Auto-approve tool calls matching a policy — the approval gate is a stream event
db.collections.permissions.subscribe(perms => {
  for (const p of perms.filter(p => p.state === 'pending')) {
    if (matchesPolicy(p)) approveViaStream(p.requestId)
  }
})

// React to cross-agent handoffs
db.collections.childSessionEdges.subscribe(edges => {
  // Agent A called Agent B — the lineage graph is in the stream
})
```

No `whileLoop()` orchestrator. No `wake()` verb. No polling. The durable stream pushes events to you. If your process crashes and restarts, replay from the last offset — nothing is lost.

---

<picture>
  <img alt="Durable Approval Flow" src="assets/durable-approval-flow.svg" width="100%">
</picture>

## Durable approval gates — approvals that survive everything

The agent calls a dangerous tool. The `approve()` middleware suspends the agent and writes a `permission_request` to the durable stream. Any subscriber — a dashboard, a Slack bot, an automated policy engine — can resolve it. The resolution lands on the stream. The conductor replays it and the agent resumes.

The key insight: the durable stream is the transport. There's no callback URL to expire, no WebSocket to drop, no in-memory state to lose. If the sandbox crashes between the request and the resolution, restart it anywhere — the approval is already on the stream, waiting to be replayed.

```typescript
// Auto-approve safe operations, escalate dangerous ones to Slack
db.collections.permissions.subscribe(perms => {
  for (const p of perms.filter(p => p.state === 'pending')) {
    if (isSafeOperation(p)) {
      approveViaStream(p.requestId)
    } else {
      postToSlack(`Agent wants to run: ${p.toolCall.command}`, p.requestId)
    }
  }
})
```

---

## Sessions survive everything

Kill a sandbox. Restart it on a different host. The session continues.

```typescript
// Host A — local dev
const handle1 = await compose(sandbox(), middleware([trace()]), agent([...]))
  .start({ serverUrl: 'http://localhost:4440', stateStream: 'my-session' })
// ... prompt a few turns, then stop

// Host B — cloud, different machine, different continent
const handle2 = await compose(sandbox(), middleware([trace()]), agent([...]))
  .start({ serverUrl: 'https://prod.internal:4440', stateStream: 'my-session' })
// session/load picks up from exactly where Host A left off
```

The session is the stream, not the sandbox. Both hosts read and write the same durable log. The agent on Host B replays the full conversation history and continues. Zero state is lost.

---

<picture>
  <img alt="Durable Wait" src="assets/durable-wait.svg" width="100%">
</picture>

## Durable waits — approvals that outlive everything

Your personal assistant agent is running in the cloud. You ask it to check your inbox. The `approve()` middleware pauses the agent and writes a `permission_request` to the durable stream. You close your laptop and walk away.

The sandbox times out after 15 minutes. The container is recycled. The agent process is gone.

Five hours later, you open the dashboard on your phone. The pending approval is right there — it's a durable stream event, not an in-memory callback. You tap "approve." The resolution lands on the stream. A new sandbox provisions, calls `session/load`, replays the stream, sees the approval, and the agent reads your email and sends you the summary.

```typescript
// Fire-and-forget personal assistant
const handle = await compose(
  sandbox({ provider: 'cloud' }),
  middleware([ trace(), approve({ scope: 'tool_calls' }), secretsProxy({...}) ]),
  agent(['claude-code-acp']),
).start({ serverUrl: 'https://prod.fireline.dev' })

// Prompt and walk away — the stream remembers everything
conn.prompt({ message: 'Check my inbox for the Acme contract and summarize it' })

// Hours later, from a different device:
// 1. Dashboard shows pending approval (it's a stream event, not a callback)
// 2. You approve
// 3. New sandbox provisions, replays stream, agent continues
```

The stream outlives the sandbox, the process, the network connection, and your attention span. That's what durable means.

---

<picture>
  <img alt="Multi-Agent Topology" src="assets/multi-agent-topology.svg" width="100%">
</picture>

## Multi-agent topologies

```typescript
import { compose, agent, sandbox, middleware, peer, fanout } from '@fireline/client'

const reviewer = compose(sandbox({...}), middleware([...]), agent(['claude-code-acp'])).as('reviewer')
const writer   = compose(sandbox({...}), middleware([...]), agent(['claude-code-acp'])).as('writer')

// peer() wires cross-agent ACP calls — type-safe at compile time
const handles = await peer(reviewer, writer).start({ serverUrl })
// handles.reviewer.acp, handles.writer.acp — each agent gets its own endpoints
// Both write to the same durable stream — one subscription sees everything

// fanout() runs N instances in parallel
const workers = await fanout(worker, { count: 3 }).start({ serverUrl })
```

---

## Examples

| Example | Pattern | What it shows |
|---|---|---|
| [`examples/flamecast-client/`](examples/flamecast-client/) | Agent platform client | `compose` + `peer` + reactive dashboard with approval flow |
| [`examples/agent-os/`](examples/agent-os/) | Multi-agent orchestration | `peer` + `fanout` + session migration + cross-agent lineage |
| [`examples/background-agent/`](examples/background-agent/) | Fire-and-forget task runner | Ramp Inspect pattern — provision, prompt, observe via stream |
| [`examples/slackbot/`](examples/slackbot/) | Integration agent | Slack Bolt + stream-based completion/approval observation |

---

## Architecture

The Rust workspace is organized by [Anthropic's managed-agent primitive taxonomy](https://www.anthropic.com/engineering/managed-agents):

```
crates/
├── fireline-semantics     Pure semantic kernel — session, approval, resume state machines
├── fireline-session        Durable-stream session log, replay, materializer, host index
├── fireline-harness        ACP conductor, middleware pipeline, approval gate, trace projector
├── fireline-sandbox        SandboxProvider trait + LocalSubprocess/Docker/Microsandbox impls
├── fireline-resources      ResourceMounter, FsBackend, resource publisher
├── fireline-tools          Peer registry, MCP tool injection, capability refs
├── fireline-orchestration  Child session edges, orchestration primitives
└── fireline-host           HTTP server, ProviderDispatcher, control plane routes
```

Proposals driving the next phase:

- [`docs/proposals/sandbox-provider-model.md`](docs/proposals/sandbox-provider-model.md) — unified SandboxProvider abstraction
- [`docs/proposals/client-api-redesign.md`](docs/proposals/client-api-redesign.md) — `compose()` client API with typed topologies
- [`docs/proposals/cross-host-discovery.md`](docs/proposals/cross-host-discovery.md) — cross-host agent discovery via durable streams
- [`docs/proposals/resource-discovery.md`](docs/proposals/resource-discovery.md) — stream-backed resource publishing + mounting
- [`docs/proposals/deployment-and-remote-handoff.md`](docs/proposals/deployment-and-remote-handoff.md) — local → cloud migration

**Formally verified.** The semantic kernel is model-checked with [TLA+](verification/spec/managed_agents.tla) and [Stateright](verification/stateright/). Key invariants: `SessionDurableAcrossRuntimeDeath`, `WakeOnReadyIsNoop`, `WakeOnStoppedChangesRuntimeId`, `ConcurrentWakeSingleWinner`, `ResourcePublishedIsEventuallyDiscoverable`.

---

## Built on

- [**durable-streams**](https://durablestreams.com) — the append-only, replayable event stream substrate
- [**ACP**](https://agentclientprotocol.com) — Agent Client Protocol for agent ↔ host communication
- [**sacp-conductor**](https://github.com/agentclientprotocol/rust-sdk) — the Rust ACP conductor + middleware pipeline
- [**microsandbox**](https://github.com/superradcompany/microsandbox) — hardware-isolated microVM sandboxes (optional provider)
- [**TanStack DB**](https://tanstack.com/db) — differential-dataflow reactive queries over materialized state

---

## License

Apache 2.0. See [LICENSE](LICENSE).
