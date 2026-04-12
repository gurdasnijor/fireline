# Agent OS — Multi-Agent Orchestration on Fireline

> Demonstrates [Rivet Agent OS](https://www.rivet.dev/agent-os/)-style patterns — multiple agents provisioned as sandboxes, cross-agent communication via ACP peer calls, durable state across agent restarts — built on Fireline's `compose` + `peer` + `@fireline/state` primitives.

## What this demonstrates

1. **Multi-agent topology** — `peer(researcher, writer)` wires two agents for cross-agent collaboration via ACP peer calls
2. **Session migration** — stop one agent, re-provision it, and the session continues from the durable stream
3. **Cross-agent lineage** — `child_session_edge` events trace which agent called which, visible in `@fireline/state`
4. **Fan-out execution** — `fanout(worker, { count: 3 })` spawns three parallel worker instances

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│  Agent OS Orchestrator (Node.js)                              │
│                                                               │
│  compose + peer + fanout + @fireline/state                    │
│                                                               │
│  ┌────────────────┐  ┌────────────────┐  ┌────────────────┐  │
│  │ Researcher      │  │ Writer          │  │ Workers (x3)    │  │
│  │ peer-calls ────────▶ receives tasks  │  │ parallel exec   │  │
│  │ traces lineage  │  │ traces lineage  │  │ fan-out pattern │  │
│  └────────┬───────┘  └────────┬───────┘  └────────┬───────┘  │
│           │                   │                    │          │
└───────────┼───────────────────┼────────────────────┼──────────┘
            └───────────────────┼────────────────────┘
                                ▼
                    Durable Streams Service
                    (one tenant stream — all agents)
```

## The code

### 1. Define agents as composable values

```typescript
// ============================================================
// Fireline — composition
// ============================================================
import { compose, agent, sandbox, middleware, peer, fanout } from '@fireline/client'
import { trace, approve, budget, inject } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

// ============================================================
// Fireline — reactive state observation
// ============================================================
import { createFirelineDB } from '@fireline/state'

// ============================================================
// ACP (third-party — NOT Fireline)
// ============================================================
import { ClientSideConnection, PROTOCOL_VERSION } from '@agentclientprotocol/sdk'

const codebase = localPath('~/projects/backend', '/workspace', true)

const researcher = compose(
  sandbox({
    resources: [codebase],
    envVars: { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY! },
    labels: { role: 'researcher', project: 'backend-review' },
  }),
  middleware([
    trace(),
    inject([{ kind: 'workspace_file', path: '/workspace/README.md' }]),
    approve({ scope: 'tool_calls' }),
  ]),
  agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
).as('researcher')

const writer = compose(
  sandbox({
    resources: [codebase],
    envVars: { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY! },
    labels: { role: 'writer', project: 'backend-review' },
  }),
  middleware([trace()]),
  agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
).as('writer')
```

### 2. Wire a peered topology

```typescript
// peer() connects agents — researcher can call writer via ACP peer calls
const topology = peer(researcher, writer)

// Provision both sandboxes with one call
const handles = await topology.start({ serverUrl: 'http://localhost:4440' })

console.log('Researcher ACP:', handles.researcher.acp.url)
console.log('Writer ACP:', handles.writer.acp.url)
console.log('State stream:', handles.researcher.state.url)
// Both agents write to the SAME tenant stream
```

### 3. Open an ACP session and start research

```typescript
// ACP session on the researcher — standard SDK, not Fireline
const ws = new WebSocket(handles.researcher.acp.url)
const conn = new ClientSideConnection(
  createHandler(),
  createWebSocketStream(ws),
)
await conn.initialize({
  protocolVersion: PROTOCOL_VERSION,
  clientInfo: { name: 'agent-os-demo', version: '0.0.1' },
  clientCapabilities: { fs: { readTextFile: false } },
})
const { sessionId } = await conn.newSession({ cwd: '/workspace' })

// Fire the research prompt — observation is via the stream
conn.prompt({
  sessionId,
  prompt: [{ type: 'text', text: 'Analyze the error handling patterns in this codebase. When you find issues, call the writer agent to draft fixes.' }],
})
```

### 4. Observe cross-agent lineage via `@fireline/state`

```typescript
// Subscribe to the shared tenant stream
const db = createFirelineDB({ stateStreamUrl: handles.researcher.state.url })
await db.preload()

// Watch for cross-agent peer calls (child_session_edge events)
db.collections.childSessionEdges.subscribe((edges) => {
  for (const edge of edges) {
    console.log(`Agent ${edge.parentRuntimeId} called agent ${edge.childRuntimeId}`)
    console.log(`  Parent session: ${edge.parentSessionId}`)
    console.log(`  Child session: ${edge.childSessionId}`)
    console.log(`  Trace: ${edge.traceId}`)
  }
})

// Watch all sessions across both agents
db.collections.sessions.subscribe((sessions) => {
  for (const session of sessions) {
    console.log(`Session ${session.sessionId} on ${session.runtimeKey}: ${session.state}`)
  }
})
```

### 5. Fan-out pattern — parallel workers

```typescript
const worker = compose(
  sandbox({
    envVars: { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY! },
    labels: { role: 'worker' },
  }),
  middleware([trace()]),
  agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
).as('worker')

// Spawn 3 parallel instances
const workerHandles = await fanout(worker, { count: 3 }).start({ serverUrl: 'http://localhost:4440' })

// Each worker gets its own sandbox, ACP endpoint, and state stream
for (let i = 0; i < workerHandles.length; i++) {
  const ws = new WebSocket(workerHandles[i].acp.url)
  const conn = new ClientSideConnection(createHandler(), createWebSocketStream(ws))
  await conn.initialize({ protocolVersion: PROTOCOL_VERSION, clientInfo: { name: `worker-${i}`, version: '0.0.1' }, clientCapabilities: {} })
  const { sessionId } = await conn.newSession({ cwd: '/' })
  conn.prompt({ sessionId, prompt: [{ type: 'text', text: `Process shard ${i} of 3` }] })
}
```

### 6. Session migration — durable across restarts

```typescript
// Stop the researcher sandbox
const { SandboxAdmin } = await import('@fireline/client')
const admin = new SandboxAdmin({ serverUrl: 'http://localhost:4440' })
await admin.destroy(handles.researcher.id)

// Re-provision with the same labels — durable stream preserves history
const newHandles = await topology.start({ serverUrl: 'http://localhost:4440' })

// Open ACP and load the existing session
const ws2 = new WebSocket(newHandles.researcher.acp.url)
const conn2 = new ClientSideConnection(createHandler(), createWebSocketStream(ws2))
await conn2.initialize({ protocolVersion: PROTOCOL_VERSION, clientInfo: { name: 'agent-os-demo', version: '0.0.1' }, clientCapabilities: {} })
await conn2.loadSession({ sessionId, cwd: '/workspace' })
// Session history is intact — replayed from the durable stream
```

## How this maps to Rivet Agent OS

| Agent OS concept | Fireline equivalent |
|---|---|
| Actor (agent instance) | `compose(sandbox, middleware, agent)` → provisioned sandbox |
| Actor state | Durable stream — `@fireline/state` materializes it reactively |
| RPC between actors | ACP peer calls — `peer(a, b)` wires the topology |
| Event streaming | Durable-streams SSE subscription via `createFirelineDB` |
| Session resumption | ACP `session/load` replays from the durable stream |
| Actor scaling | `fanout(harness, { count: N })` — N parallel instances |
| Deployment portability | `compose().start({ serverUrl })` — same code, any server URL |

## Key difference from Agent OS

Rivet's Agent OS uses V8 isolates for ~6ms cold starts. Fireline uses subprocess/Docker/microsandbox providers for heavier isolation. The trade-off: Fireline agents get full OS-level tool access (file system, network, shell) at the cost of higher boot time (~300ms-3s depending on provider). For code-review and engineering agents that need real shells and file systems, this is the right trade-off.

## Coming soon

- **Cross-host deployment** — `peer(agent.at('us-east'), agent.at('eu-west'))` ([`docs/proposals/cross-host-discovery.md`](../../docs/proposals/cross-host-discovery.md))
- **Stream-FS** — live filesystem snapshots across agents ([`docs/proposals/stream-fs-spike.md`](../../docs/proposals/stream-fs-spike.md))
- **Pooled providers** — pre-warmed sandboxes for sub-100ms provisioning ([`docs/proposals/sandbox-provider-model.md`](../../docs/proposals/sandbox-provider-model.md) §5)
