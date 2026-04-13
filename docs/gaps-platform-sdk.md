# Platform SDK Gaps

> Fireline as a platform: imperative APIs for building applications on top of Fireline — dashboards, Slackbots, orchestrators, custom agent UIs.
>
> Consumer: application developers who need granular control over sessions, state, approvals, and file access.
>
> Companion doc: [`gaps-declarative-agent-api.md`](gaps-declarative-agent-api.md) — declarative/CLI gaps for defining and running agents.
>
> Evidence base: [`investigations/package-api-ergonomics-gaps.md`](investigations/package-api-ergonomics-gaps.md) — grounded in the Flamecast port with line-number citations.
>
> Date: 2026-04-13

---

## The target API

One package. Two concerns: agents (scoped) and state (global).

```typescript
import fireline, { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, approve } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

// 1. State — global, sees everything, not agent-scoped
const db = await fireline.db()

// 2. Define + start agents
const reviewer = await compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([trace(), approve({ scope: 'tool_calls' })]),
  agent(['claude-acp']),
).as('reviewer').start()

const writer = await compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([trace()]),
  agent(['claude-acp']),
).as('writer').start()

// 3. Connect to an agent — returns the ACP SDK's ClientSideConnection
const conn = await reviewer.connect()
const { sessionId } = await conn.newSession({ cwd: '/workspace', mcpServers: [] })
await conn.prompt({ sessionId, prompt: [{ type: 'text', text: 'Review this repo.' }] })

// 4. Query state — all agents, all sessions, one unified view
db.sessions        // reviewer + writer sessions
db.promptRequests // all requests across all agents

// 5. React to approval requests (subscribe, don't poll)
db.permissions.subscribe(rows => {
  const pending = rows.find(p => p.sessionId === sessionId && p.state === 'pending')
  if (pending) {
    // Resolve — the conductor is durably waiting on the stream
    reviewer.resolvePermission(sessionId, pending.requestId, { allow: true })
  }
})

// 6. Lifecycle
await reviewer.stop()
```

`fireline.db()` connects to the local durable stream. No URL — the environment determines where the stream is (`FIRELINE_STREAM_URL` for remote). The DB sees all agents, all sessions, all state. It's not scoped to any single agent.

The agent object (`reviewer`, `writer`) is scoped: `connect()` opens ACP to that agent, `resolvePermission()` appends to that agent's stream, `stop()` stops that agent. But the DB is the unified view across everything.

What changes from today:

| Today | Target |
|---|---|
| `import { createFirelineDB } from '@fireline/state'` | `const db = await fireline.db()` — no separate package, no URL |
| `start({ serverUrl: '...' })` returns a data record | `start()` returns a live agent object — no URL needed |
| `openNodeAcpConnection(handle.acp.url, ...)` — 39 LOC | `reviewer.connect()` → ACP SDK `ClientSideConnection` |
| `createFirelineDB({ stateStreamUrl: handle.state.url })` | `fireline.db()` — implicit stream, env-configured |
| Raw `DurableStream.append(JSON.stringify(...))` | `reviewer.resolvePermission(sessionId, requestId, outcome)` |
| ad-hoc wait helpers in older examples | `db.permissions.subscribe(rows => ...)` — current direct pattern |
| `import { Sandbox } from '../../packages/client/src/...'` | `import { ... } from '@fireline/client'` |

---

## The core problems

### Problem 1: `start()` returns a data record, not something you work with

The current `start()` returns a `SandboxHandle` — four fields (`id`, `provider`, `acp.url`, `state.url`). It's a provision receipt. Every app has to take those raw URLs and manually assemble ACP connections, state databases, and approval workflows from scratch.

The fix: `start()` returns a live `FirelineAgent` object. It owns the transport, the state connection, and the lifecycle. The raw URLs are still accessible as escape hatches.

### Problem 2: Observation is a separate package with its own bootstrap

Today, using Fireline requires importing from three packages:

```typescript
import { compose, agent, sandbox, middleware } from '@fireline/client'      // control
import { createFirelineDB } from '@fireline/state'                          // observation
import { useLiveQuery } from '@tanstack/react-db'                           // also observation?
import { eq } from '@tanstack/db'                                           // and this too?
```

From the user's perspective, there's one thing: `@fireline/client`. The fact that observation lives in a separate npm package with its own types, its own bootstrap, and its own TanStack dependency is an implementation detail leaking into the API.

The fix: `fireline.db()` is the entry point. No separate import. `@fireline/state` can remain as the internal implementation, but the public paths are:

- `@fireline/client` — `fireline.db()`, compose, start, connect, resolve
- `@fireline/client/middleware` — trace, approve, budget, secretsProxy, peer
- `@fireline/client/resources` — localPath, gitRepo, etc.
- `@fireline/client/react` — React bindings (re-exports from state internals)

The stream location is infrastructure config, not application code. `fireline.db()` connects to the local embedded stream by default. `FIRELINE_STREAM_URL` env var overrides for remote. The code never changes between environments.

TanStack DB / [StreamDB](https://durablestreams.com/stream-db) is the internal implementation. Users who want raw `useLiveQuery` can opt in, but the default path doesn't require knowing about it.

### Evidence

The Flamecast port required **2,262 lines of glue** to bridge "sandbox provisioned" → "working application." Most of that glue is assembling the three packages into one coherent surface.

---

## P1. `agent.connect()` — ACP transport bridge

**What exists:** `SandboxHandle` returns raw `acp.url`. The ACP SDK ships [`ClientSideConnection`](https://github.com/agentclientprotocol/typescript-sdk/blob/main/src/acp.ts#L531) with `newSession`, `prompt`, `loadSession`, `cancel`. But there's no bridge from Fireline to an initialized connection.

**What apps still write today:** 39 lines of WebSocket setup, stream bridging, and stubbed Client methods when they need a raw ACP bridge. The old shared helper is gone, but app-local copies still exist where `agent.connect()` would remove the glue entirely.

**Target:**
```typescript
const conn = await agent.connect()

// conn IS a ClientSideConnection — full ACP SDK, nothing wrapped
const { sessionId } = await conn.newSession({ cwd: '/workspace', mcpServers: [] })
await conn.prompt({ sessionId, prompt: [{ type: 'text', text: '...' }] })
await conn.loadSession({ sessionId, cwd: '/workspace', mcpServers: [] })
await conn.cancel({ sessionId })
```

Fireline adds the transport setup only: endpoint → WebSocket → initialized `ClientSideConnection`. After that, you're in the ACP SDK's world. Works for Node, browser (without React), server-side orchestrators. React apps can still use `use-acp` directly.

**Evidence:** `examples/flamecast-client/shared/acp-node.ts:29-63`, `examples/flamecast-client/server.ts:447-449`.

**Work:** ~40 LOC.

---

## P2. `fireline.db()` — unified state across all agents

**What exists:** `createFirelineDB()` in a separate `@fireline/state` package, requires a raw stream URL, manual `preload()`, manual teardown.

**Target:**
```typescript
import fireline from '@fireline/client'

const db = await fireline.db()

// Global view — all agents, all sessions
db.sessions
db.promptRequests
db.permissions
db.chunks
```

No URL. No separate package import. `fireline.db()` connects to the local embedded durable stream by default. For remote environments, `FIRELINE_STREAM_URL` env var overrides — the code never changes.

React companion:
```typescript
import { FirelineProvider, useDb } from '@fireline/client/react'

<FirelineProvider>
  <App />
</FirelineProvider>
```

**Evidence:** `examples/flamecast-client/ui/provider.tsx:22-45` — custom config endpoint, manual preload, manual teardown.

**Work:** ~20 LOC. Wraps `createFirelineDB` internally, reads stream URL from env or defaults to local.

---

## P3. Approvals are already durable — the TS side just needs to write to the stream

The Rust conductor already implements durable approval waits (`crates/fireline-harness/src/approval.rs`):

1. **`emit_permission_request`** (line 254) — writes `permission_request` to the durable stream
2. **`wait_for_approval`** (line 294) — opens a live SSE reader on the stream and blocks the prompt in the conductor pipeline until `approval_resolved` appears
3. **`rebuild_from_log`** (line 176) — on `session/load`, replays the stream from the beginning, finds whether a prior approval already resolved

The conductor is both producer and consumer. The stream is the rendezvous point. If the sandbox dies and a new one boots, `session/load` replays the stream, finds the resolution, and the agent continues. This is a real durable wait — not an in-memory Promise.

The TypeScript side doesn't need a `waitFor` primitive. It needs exactly one thing: a way to append `approval_resolved` to the stream. That's P4.

For observation (dashboards, Slack bots, etc.), the standard pattern is **subscribe to the collection** and react to changes. The `ObservableCollection.subscribe()` / `subscribeChanges()` methods already exist on `FirelineDB`. No new primitive needed — just use subscribe.

---

## P4. Approval resolution — exists but buried

**What already exists:** `packages/client/src/events.ts` exports `appendApprovalResolved()` — a clean 32-line helper that appends `approval_resolved` to the durable stream with the correct envelope format. **It is NOT re-exported from `packages/client/src/index.ts`.** So every example reimplements it.

**What `@fireline/state` already exports:** `createPendingPermissionsCollection()` — a live reactive query over pending permissions. Also `createSessionPermissionsCollection()` for session-scoped views. **No example uses either.**

The full approval lifecycle already works:

```typescript
// Declare (compose-time)
middleware([approve({ scope: 'tool_calls' })])

// Observe (runtime) — createPendingPermissionsCollection ALREADY EXISTS
const pending = createPendingPermissionsCollection({ permissions: db.permissions })
pending.subscribe(rows => {
  for (const p of rows) {
    // Resolve — appendApprovalResolved ALREADY EXISTS (just not re-exported)
    appendApprovalResolved({ streamUrl, sessionId: p.sessionId, requestId: p.requestId, allow: true })
  }
})
```

**The actual gaps:**
1. `appendApprovalResolved` is not re-exported from `@fireline/client` root — **1 line fix**
2. Examples don't use `createPendingPermissionsCollection` — **documentation gap**
3. Ideally `appendApprovalResolved` would move onto the `FirelineAgent` object as `agent.resolvePermission()` — **ergonomic improvement, ~10 LOC**

**Additional issue:** `approve({ scope: 'tool_calls' })` reads as if tool-call-scoped approval works, but the client maps it to a prompt-level gate fallback (`sandbox.ts:179-197`). This semantic drift should be documented or fixed.

---

## P5. Session-centric selectors — already exist, completely unused

**What already exists in `@fireline/state`:**

| Builder | File | What it does |
|---|---|---|
| `createSessionTurnsCollection` | `collections/session-turns.ts` | Live view of turns for a session, sorted by `startedAt` |
| `createTurnChunksCollection` | `collections/turn-chunks.ts` | Live view of chunks for a turn, sorted by `seq` |
| `createPendingPermissionsCollection` | `collections/pending-permissions.ts` | Live view of all pending permissions |
| `createSessionPermissionsCollection` | `collections/session-permissions.ts` | Live view of permissions for a session |
| `createQueuedTurnsCollection` | `collections/queued-turns.ts` | Live view of queued turns |
| `createActiveTurnsCollection` | `collections/active-turns.ts` | Live view of active turns |

**These are exported and completely ignored by every example.** Instead, examples hand-write 10-line join/filter/sort patterns.

**The gap is documentation and usage, not code.** When we unify `@fireline/state` into `@fireline/client`, these should be accessible from `fireline.db()`.

**Evidence:** `examples/flamecast-client/ui/hooks/use-session-state.ts:46-82` — 36 lines of manual join that `createSessionTurnsCollection` + `createTurnChunksCollection` already handle.

---

## P6. File browsing for running sandboxes

**What exists:** `resources.ts` defines mounts at provisioning time. No runtime file inspection.

**What the Flamecast port wrote:** 170+ lines of custom routes for file preview, directory listing, git branches, git worktrees.

**Decision needed:** What is the canonical substrate?
- ACP's `readTextFile` (already on `ClientSideConnection`) for file content
- A Fireline-provided directory listing API for structure
- State-projected file events on the durable stream for observation

For now, `conn.readTextFile({ sessionId, path })` from the ACP SDK covers the file content case. Directory listing is the gap.

**Evidence:** `examples/flamecast-client/ui/fireline-client.ts:197-223`, `259-376`, `server.ts:220-287`.

**Work:** Design decision + ~40 LOC.

---

## P7. Admin surface extensions

**What exists:** `SandboxAdmin` has `get`, `list`, `destroy`, `status`, `healthCheck`.

**What operator apps need:**
- `stop(id)` — graceful stop without deletion
- `waitUntilReady(id, { timeoutMs })` — poll/subscribe until sandbox is ready
- Server-side label filters (current `list()` filters client-side)

These could also live on the agent object directly:
```typescript
await agent.stop()       // graceful stop
await agent.destroy()    // full teardown
agent.status             // live, from state stream
```

**Work:** ~40 LOC.

---

## P8. Package consumption — examples use published packages

**Current:** `import { Sandbox } from '../../packages/client/src/sandbox.ts'`

**Target:** `import { compose, agent, sandbox } from '@fireline/client'`

**Work:** Workspace dependency wiring + stable subpath exports. Build config, not code.

---

## Summary

| Gap | Work | Eliminates | Priority |
|-----|------|-----------|----------|
| **P1 `agent.connect()`** | ~40 LOC | app-local ACP bridge helpers | Must have |
| **P2 `fireline.db()`** | ~20 LOC | Separate `@fireline/state` import, raw URLs, manual preload | Must have |
| **P3 Durable approvals** | 0 LOC (Rust done) | Approval wait is already durable in conductor | Documented |
| **P4 Approval resolution** | **1 line** (re-export) + ~10 LOC (agent method) | `appendApprovalResolved` exists in events.ts, not re-exported | Must have |
| **P5 Session selectors** | **0 LOC** (already exist) | 6 collection builders exported from `@fireline/state`, unused | Documentation |
| P6 File browsing | ~40 LOC + design | 170 LOC of custom routes in Flamecast | Needs design |
| P7 Admin extensions | ~40 LOC | Custom lifecycle management | Nice to have |
| P8 Package consumption | Build config | Raw source imports in examples | Must have |

## Implementation plan

### Phase 1 — The agent object + `fireline.db()` (~110 LOC)

Revise `start()` to return a live agent. Add `fireline.db()` as the global state entry point.

```typescript
// The agent — scoped to one sandbox
interface FirelineAgent {
  // Identity
  readonly id: string
  readonly provider: string

  // Raw endpoints (escape hatch)
  readonly acp: Endpoint
  readonly state: Endpoint

  // P1: ACP transport bridge → returns ACP SDK ClientSideConnection
  connect(clientName?: string): Promise<ClientSideConnection>

  // P4: Approval resolution — appends to the durable stream
  resolvePermission(sessionId: string, requestId: string, outcome: { allow: boolean }): Promise<void>

  // Lifecycle
  stop(): Promise<void>
  destroy(): Promise<void>
}

// The DB — global, sees all agents
// fireline.db() returns this. Internally wraps @fireline/state's createFirelineDB.
interface FirelineDB {
  sessions: ObservableCollection<SessionRow>
  promptRequests: ObservableCollection<PromptRequestRow>
  permissions: ObservableCollection<PermissionRow>
  chunks: ObservableCollection<ChunkRow>
  // ... other collections

  close(): void
}
```

`start()` with no arguments boots locally (conductor in-process, agent via stdio, embedded durable streams). `fireline.db()` connects to the local embedded stream by default; `FIRELINE_STREAM_URL` env var overrides for remote.

### Phase 2 — Session selectors (~40 LOC)

Document existing query builders in `@fireline/state`. Add convenience methods on `FirelineDB`: `sessionTranscript`, `pendingPermissions`, `sessionStatus`.

### Phase 3 — Package consumption (build config)

Wire examples as workspace consumers of `@fireline/client`. Delete raw source path imports. `@fireline/state` becomes an internal dependency, not a public import.

---

The old `examples/shared/` cleanup is already landed on `main`. After Phase 1-3, the remaining per-app glue drops to Flamecast-specific UI adaptation only.

**Total Phase 1 work: ~110 LOC TypeScript, zero Rust.**
