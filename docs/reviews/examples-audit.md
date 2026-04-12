# Examples Audit — `@fireline/client` Idiomatic Usage

> Audit of `examples/` against the idiomatic patterns in today's
> `@fireline/client` surface (see
> [`packages/client/src/index.ts`](../../packages/client/src/index.ts)).
>
> Date: 2026-04-12

## Summary

| Example | Idiomatic | Uses deleted helpers? | Blocking issues |
|---|---:|---|---|
| [`live-monitoring/`](../../examples/live-monitoring/index.ts) | **100%** | no | none |
| [`background-task/`](../../examples/background-task/index.ts) | **~90%** | no | `connectAcp` instead of `handle.connect()` |
| [`code-review-agent/`](../../examples/code-review-agent/index.ts) | **~85%** | no | no `stop()`, `connectAcp` direct |
| [`cross-host-discovery/`](../../examples/cross-host-discovery/index.ts) | **~85%** | no | no `stop()`, `connectAcp` direct |
| [`multi-agent-team/`](../../examples/multi-agent-team/index.ts) | **~80%** | no | no `stop()` on either agent, `connectAcp` direct |
| [`crash-proof-agent/`](../../examples/crash-proof-agent/index.ts) | **~70%** | no | `SandboxAdmin.destroy()` instead of `first.stop()` |
| [`approval-workflow/`](../../examples/approval-workflow/index.ts) | **~70%** | no | uses `appendApprovalResolved` when the agent object is in scope (should be `handle.resolvePermission()`) |
| [`flamecast-client/`](../../examples/flamecast-client/) | **~40%** (server ~40%, UI ~95%) | **yes** (local copies of deleted helpers) | relative-path imports into `packages/`, `openNodeAcpConnection`, `resolveApproval`, `SandboxAdmin`, `envVars: { ANTHROPIC_API_KEY }`, untyped `provider: string` |

**Deleted helpers still present anywhere?** Only in
`examples/flamecast-client/shared/acp-node.ts` and
`examples/flamecast-client/shared/resolve-approval.ts` — these are local
copies inside `flamecast-client/`, not the root `examples/shared/` (which
was correctly pruned to just `wait.ts`).

---

## `code-review-agent/index.ts` — ~85%

**What it demonstrates.** Local-repo code review with approval gates and
optional `ANTHROPIC_API_KEY` injection via `secretsProxy`.

**Idiomatic ✓:**
- L1: `import fireline, { agent, compose, connectAcp, middleware, sandbox } from '@fireline/client'` — unified import surface
- L12-14: `secretsProxy({ ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' } })` — correct pattern for API keys (NOT `envVars`)
- L21: `fireline.db({ stateStreamUrl: handle.state.url })`
- L25: `db.permissions` — flattened collection access

**Non-idiomatic:**
- L22: `connectAcp(handle.acp, 'code-review-agent')` — should be
  `handle.connect('code-review-agent')`. `handle` is a `FirelineAgent`;
  `.connect()` is the idiomatic method.
- L16-20: variable named `handle` but the value is a `FirelineAgent`.
  Cosmetic, but naming it `agent`/`reviewer` makes the `.connect()` /
  `.stop()` methods read better.
- L27: `await acp.close(); db.close()` — no `handle.stop()`. The sandbox
  is left running after the script exits. Should add
  `await handle.stop()`.

**Deleted helpers used?** No.

---

## `crash-proof-agent/index.ts` — ~70%

**What it demonstrates.** Session survives sandbox death — provision on
primary, destroy, `session/load` on a different host, continue.

**Idiomatic ✓:**
- L1: import from `@fireline/client`
- L16: `fireline.db({...})`
- L20: `db.promptTurns` — flattened
- Named `first`/`second` (good)

**Non-idiomatic:**
- **L2, L14: `import { SandboxAdmin } from '@fireline/client/admin'` +
  `new SandboxAdmin({ serverUrl: primaryUrl }).destroy(first.id)`** —
  the whole point of the `FirelineAgent` object is to replace this. Fix:
  ```ts
  // remove the SandboxAdmin import
  await first.stop()
  ```
  That's the intentional first-host-dies moment; `first.stop()` says
  exactly what the scenario says.
- L11, L17: `connectAcp(first.acp, ...)` / `connectAcp(second.acp, ...)`
  — should be `first.connect(...)` / `second.connect(...)`.
- L22: no `second.stop()` — leaks the rescue sandbox.

**Deleted helpers used?** No.

---

## `approval-workflow/index.ts` — ~70%

**What it demonstrates.** Durable approvals via an external HTTP
webhook — pending permission → POST to local broker → stream resolution
→ agent continues.

**Idiomatic ✓:**
- L1: `appendApprovalResolved` imported from `@fireline/client` root
  (not `@fireline/client/events`)
- L12-18: secretsProxy for ANTHROPIC_API_KEY
- L24: `fireline.db({...})`
- L25: `db.permissions.subscribe` — flattened
- Uses an external "broker" pattern to model the dashboard/webhook case
  cleanly.

**Non-idiomatic:**
- **L10: `appendApprovalResolved({ streamUrl, sessionId, requestId,
  allow: true, ... })` inside a broker that runs in the same process as
  `handle`.** The idiomatic rule is: if you have the `FirelineAgent`,
  use `handle.resolvePermission(...)`; reserve `appendApprovalResolved`
  for when the resolver has no handle (a different process, a Slack
  bot, a Lambda). Here, `streamUrl` is captured from `handle.state.url`
  in the same closure — use:
  ```ts
  await handle.resolvePermission(body.sessionId, body.requestId, {
    allow: true,
    resolvedBy: 'approval-workflow',
  })
  ```
  The example can be rewritten to use `appendApprovalResolved` *from a
  different process* to genuinely demonstrate the "external resolver"
  pattern — right now it uses the external-pattern helper in an
  internal context, which muddies the point.
- L26: `connectAcp(handle.acp, ...)` → `handle.connect(...)`.
- L31: no `handle.stop()` — leaks the sandbox.
- L19-23: `handle` name (cosmetic).

**Deleted helpers used?** No.

---

## `background-task/index.ts` — ~90%

**What it demonstrates.** Fire-and-forget: prompt an agent, exit the
runner, later re-open the stream to observe completion. The observation
mode is a neat test of `fireline.db()` without a control plane.

**Idiomatic ✓:**
- L1: unified imports
- L6-8: observation-only branch — `fireline.db({ stateStreamUrl })`,
  then `db.sessions.toArray`, `db.promptTurns.toArray` — flattened
- L13-18: conditional secretsProxy for `ANTHROPIC_API_KEY`
- No `stop()` at exit is **intentional** — this is the fire-and-forget
  pattern; the sandbox is supposed to outlive the runner. Appropriate
  here.

**Non-idiomatic:**
- L24: `connectAcp(handle.acp, ...)` → `handle.connect(...)`.
- L19-23: `handle` name (cosmetic).

**Deleted helpers used?** No.

---

## `live-monitoring/index.ts` — 100%

**What it demonstrates.** Browser-only observation UI — live query
across sessions, prompt turns, permissions, and tool-call chunks.

**Idiomatic ✓:**
- L2: `import fireline, { type FirelineDB } from '@fireline/client'`
- L18: `void fireline.db({ stateStreamUrl }).then(...)` with cleanup on
  unmount
- L41-44: `db.sessions`, `db.promptTurns`, `db.permissions`, `db.chunks`
  — flattened in all four `useLiveQuery` calls
- L45: `useAcpClient` from `use-acp` — correct React ACP client
- L51: `acp.resolvePermission(...)` via the ACP SDK — this resolves the
  pending permission on the local ACP connection, not the stream, which
  is the right tool for an interactive UI

**Non-idiomatic:** None.

**Deleted helpers used?** No.

---

## `multi-agent-team/index.ts` — ~80%

**What it demonstrates.** Researcher → writer pipeline with `pipe(...)`;
observes completion via `db.promptTurns`, joins chunks via
`db.chunks.toArray`.

**Idiomatic ✓:**
- L1: `fireline` default + `pipe` from the root
- L11: `fireline.db({ stateStreamUrl: handles.researcher.state.url })`
  — uses one shared stream across the pipeline
- L17, L21: `db.chunks.toArray`, `db.promptTurns.toArray`,
  `db.sessions.toArray`, `db.childSessionEdges.toArray` — flattened

**Non-idiomatic:**
- L12-13: `connectAcp(handles.researcher.acp, ...)` /
  `connectAcp(handles.writer.acp, ...)` → `handles.researcher.connect()`
  / `handles.writer.connect()`.
- L22: `await researcher.close(); await writer.close(); db.close()` —
  closes ACP connections but not the sandboxes. Should call
  `await handles.researcher.stop(); await handles.writer.stop()`.
- Consider naming `stage(name)` locals `reviewer`/`writer` consistently
  — the current code mixes `researcher`/`writer` ACP connections with a
  `researcher` stage variable.

**Deleted helpers used?** No.

---

## `cross-host-discovery/index.ts` — ~85%

**What it demonstrates.** Two control planes (ports 4440 and 5440) with
shared deployment discovery; `agent-b` lists and prompts `agent-a`
through the peer MCP surface.

**Idiomatic ✓:**
- L2: `import fireline, { ..., type FirelineDB } from '@fireline/client'`
- L9-12: clean `startHarness()` helper; names `agentA`, `agentB`
  (matches `FirelineAgent` returned by `.start()`)
- L13: `fireline.db(...)`
- L24: `middleware([peer()])` — middleware-level peer component
- L38, L40-41: `db.chunks.toArray`, `db.promptTurns.toArray`,
  `db.promptTurns.subscribe`, `db.chunks.subscribe` — flattened

**Non-idiomatic:**
- L14: `connectAcp(agentB.acp, 'cross-host-discovery')` →
  `agentB.connect('cross-host-discovery')`.
- L20: `acp.close(); db.close()` — no `agentA.stop()` / `agentB.stop()`.
  Both sandboxes leak.

**Deleted helpers used?** No.

---

## `flamecast-client/` — server ~40%, UI ~95%

**What it demonstrates.** A Flamecast-like platform: UI + HTTP API
server over `@fireline/client`, with session management, permissions,
runtimes, file-system snapshots.

### `flamecast-client/server.ts` — ~40%

**Critical non-idiomatic patterns:**

- **L7-11: relative-path imports into `packages/`.** The server pulls
  types from `../../packages/state/src/index.ts` and all of
  `@fireline/client` via `../../packages/client/src/*.ts`. This
  bypasses the published package surface and couples the example to the
  internal source tree.
  ```ts
  // Today:
  import { type ChunkRow, type PermissionRow, type PromptTurnRow } from "../../packages/state/src/index.ts";
  import { db as openFirelineDb, compose, agent, middleware, sandbox, type FirelineDB } from "../../packages/client/src/index.ts";
  import { SandboxAdmin } from "../../packages/client/src/admin.ts";
  import { approve, trace } from "../../packages/client/src/middleware.ts";
  import { localPath } from "../../packages/client/src/resources.ts";
  // Should be:
  import type { ChunkRow, PermissionRow, PromptTurnRow } from "@fireline/state";
  import fireline, { compose, agent, middleware, sandbox, type FirelineDB } from "@fireline/client";
  import { SandboxAdmin } from "@fireline/client/admin";
  import { approve, trace } from "@fireline/client/middleware";
  import { localPath } from "@fireline/client/resources";
  ```

- **L12-13, and `flamecast-client/shared/{acp-node,resolve-approval}.ts`:
  local copies of the deleted helpers.** The root
  `examples/shared/acp-node.ts` and `examples/shared/resolve-approval.ts`
  were correctly deleted; flamecast re-introduced them as
  `examples/flamecast-client/shared/`. These should be removed:
  - `openNodeAcpConnection(handle.acp.url, ...)` (L447) →
    `handle.connect(...)` or `connectAcp(handle.acp, ...)` from
    `@fireline/client`
  - `resolveApproval(stateStreamUrl, sessionId, requestId, allow)` (L498,
    L618) → if the `FirelineAgent` is in scope, use
    `record.agent.resolvePermission(sessionId, requestId, { allow })`.
    If only the stream URL is available, use `appendApprovalResolved`
    from `@fireline/client` (same signature as `resolveApproval` but
    without the copy-pasted `DurableStream` glue).

- **L74-76, L635: `sharedEnv = { ANTHROPIC_API_KEY: ... }` passed via
  `envVars`.** This is the pre-`secretsProxy` pattern. Fix:
  ```ts
  // Remove sharedEnv entirely. In provisionSandbox:
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    ...(process.env.ANTHROPIC_API_KEY
      ? [secretsProxy({ ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' } })]
      : []),
  ])
  ```

- **L78, L486: `admin.destroy(record.sandboxId)`.** Store the
  `FirelineAgent` in `SessionRecord` instead of the raw handle, and call
  `record.agent.stop()`. Only keep `SandboxAdmin` for cases where the
  server receives a sandbox id from elsewhere (not the normal session
  lifecycle).

- **L633: `sandbox({ provider: options.provider, ... })` with
  `provider: string`.** `SandboxProviderConfig` is a discriminated
  union now; the string fails type-level checks. Either narrow
  `template.runtime.provider` to the union type (`'local' | 'docker' |
  'microsandbox' | 'anthropic'`) and branch, or accept only known
  providers in the template schema.

- **L52, L447, L472-474: `record.connection.connection.prompt(...)`** —
  the double `.connection` is a symptom of wrapping a
  `ClientSideConnection` in the old `openNodeAcpConnection` return
  shape. `connectAcp()` / `handle.connect()` both return a
  `ConnectedAcp` which IS a `ClientSideConnection` (with a `.close()`
  method added), so the inner call becomes `record.acp.prompt(...)`.

**Idiomatic ✓:**
- `compose(sandbox(...), middleware([...]), agent([...])).start({...})`
  — pattern is correct
- `stateStream: sharedStateStream` — shared stream across sessions
- Observability uses `stateDb.permissions.toArray` — flattened (L605)

### `flamecast-client/ui/fireline-client.ts` — ~60%

- **L1-2: `../../../packages/client/src/{sandbox,admin}.ts`** — same
  relative-path-into-source issue. Use `@fireline/client` and
  `@fireline/client/admin`.
- The rest of the file is a typed HTTP client to the server's REST
  surface — not Fireline surface, so no other patterns apply.

### `flamecast-client/ui/provider.tsx` — ~100%

- L2: `import { db as openFirelineDb, type FirelineDB } from "@fireline/client"` ✓
- L29: `await openFirelineDb({ stateStreamUrl: config.stateStreamUrl })` ✓
- Clean cancellation handling on unmount.

### `flamecast-client/ui/hooks/use-session-state.ts` — ~100%

- L5: `import type { ChunkRow, PermissionRow, PromptTurnRow } from "@fireline/state"` ✓
- L3, L4: `useLiveQuery` from `@tanstack/react-db`, `useAcpClient` from
  `use-acp`
- L7: `useFirelineDb()` hook wrapping the provider context ✓

The UI side of flamecast is well-migrated. The server side is the
laggard — it was written before `FirelineAgent`, `connectAcp`, and
`secretsProxy` landed, and never caught up.

---

## Cross-cutting findings

1. **`connectAcp(handle.acp, name)` is everywhere.** Six of eight
   examples use the standalone function when the `FirelineAgent`
   returned from `.start()` already has a `.connect(name)` method.
   Preferred pattern:
   ```ts
   const agent = await compose(...).start({ serverUrl })
   const acp = await agent.connect('my-client')
   ```
   `connectAcp` remains correct for cases where only an `Endpoint` is
   in hand (e.g., loading a URL from config) — but when we just made
   the agent, use the method.

2. **`handle` variable naming persists.** The name was accurate when
   `.start()` returned a `SandboxHandle`. It now returns a
   `FirelineAgent`. Rename to `agent`, `reviewer`, `assistant`, etc. —
   the calling code then reads `agent.connect()`, `agent.stop()`,
   `agent.resolvePermission(...)` naturally.

3. **Missing `agent.stop()` at exit.** Five of the seven
   provisioning-style examples leak sandboxes by not stopping them.
   Only `background-task` and the observation-only `live-monitoring`
   justify this; the rest should clean up.

4. **`approval-workflow` uses the external-resolver helper in an
   internal context.** Either switch to `handle.resolvePermission()`,
   or restructure so the broker actually runs in a separate process
   (which is what the example claims to demonstrate).

5. **`flamecast-client/server.ts` is the clear outlier.** It pre-dates
   `FirelineAgent`, `secretsProxy`, `connectAcp`, the provider
   discriminated union, and the published-package surface. Every other
   non-idiomatic pattern in this audit shows up there at least once.
   The UI side is already migrated; the server needs a pass.

## Recommended sweep

- **Cheap fixes (5 minutes per example):** rename `handle` → `agent`,
  swap `connectAcp(agent.acp, ...)` → `agent.connect(...)`, add
  `await agent.stop()` at the bottom. Covers 6 of 8 examples.
- **`approval-workflow`:** flip to `handle.resolvePermission()` inside
  the broker, or genuinely externalize the broker. Pick one.
- **`crash-proof-agent`:** delete the `SandboxAdmin` import; use
  `first.stop()`.
- **`flamecast-client/server.ts`:** the biggest lift —
  `@fireline/client` imports, `secretsProxy` replacing `envVars`,
  `FirelineAgent` replacing `SandboxAdmin.destroy`,
  `handle.resolvePermission()` replacing `resolveApproval()`, typed
  `SandboxProviderConfig` replacing `provider: string`, and removal of
  the local `shared/acp-node.ts` / `shared/resolve-approval.ts`
  re-implementations.
