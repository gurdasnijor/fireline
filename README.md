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

```bash
git clone https://github.com/fireline/fireline && cd fireline
pnpm install
pnpm --filter @fireline/browser-harness dev
# Open http://localhost:5173 — click Launch Agent
```

One command builds the Rust server, starts an embedded durable-streams service, boots the control plane, and opens a browser harness. No Docker. No external services.

---

## The three planes

| Plane | Package | What it does |
|---|---|---|
| **Control** | `@fireline/client` | `compose(sandbox, middleware, agent).start()` — provision sandboxes, wire middleware, execute commands |
| **Session** | `@agentclientprotocol/sdk` | ACP over `handle.acp.url` — `newSession()`, `prompt()`, `loadSession()`. Third-party protocol; Fireline never wraps it. |
| **Observation** | `@fireline/state` | `createFirelineDB({ stateStreamUrl: handle.state.url })` → `useLiveQuery()` — reactive TanStack DB collections over the durable stream |

The control plane gives you a handle. The handle carries two endpoints. Each endpoint connects you to a different plane. No side channels.

---

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
