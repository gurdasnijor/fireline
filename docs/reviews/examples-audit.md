# Examples Audit

Audit date: 2026-04-12

Scope:
- `examples/code-review-agent/`
- `examples/crash-proof-agent/`
- `examples/approval-workflow/`
- `examples/background-task/`
- `examples/live-monitoring/`
- `examples/multi-agent-team/`
- `examples/cross-host-discovery/`
- `examples/flamecast-client/`

Reference surface:
- `packages/client/src/index.ts`

Scoring:
- `10/10` = idiomatic current-main usage
- `7-9/10` = mostly idiomatic, one notable drift
- `4-6/10` = mixed, several outdated patterns
- `0-3/10` = fighting the library surface

Note:
- `flamecast-client` is currently dirty in the worktree and appears mid-migration. Findings below reflect the on-disk snapshot I reviewed, not a stable committed state.
- The "deleted-helper imports remaining" column counts imports of the explicitly deleted ACP/approval helpers. It does not count other helper indirections like `examples/shared/wait.ts`.

## Summary

| Example | Idiomatic score | Deleted-helper imports remaining | Biggest issue |
|---|---:|---:|---|
| `code-review-agent` | 7/10 | 0 | Uses `connectAcp()` and `waitForRows()` instead of the returned `FirelineAgent` and direct state subscriptions |
| `crash-proof-agent` | 4/10 | 0 | Bypasses the live agent object with both `connectAcp()` and `SandboxAdmin.destroy()` |
| `approval-workflow` | 3/10 | 0 | Resolves approvals with raw `appendApprovalResolved()` even though the example owns the launched agent |
| `background-task` | 8/10 | 0 | Still calls `connectAcp(handle.acp, ...)` instead of `handle.connect()` |
| `live-monitoring` | 9/10 | 0 | Code is clean; the README still documents obsolete `db.collections.*` access |
| `multi-agent-team` | 5/10 | 0 | Uses `connectAcp()` and helper waiting instead of the live agent objects and direct subscriptions |
| `cross-host-discovery` | 8/10 | 0 | Uses `connectAcp()` even though `start()` already returned a `FirelineAgent` |
| `flamecast-client` | 2/10 | 0 | Rebuilds custom REST/session infrastructure instead of using Fireline's control, session, and observation planes |

## code-review-agent

- Medium: `examples/code-review-agent/index.ts:16-22` treats the result of `start()` as a plain handle and then calls `connectAcp(handle.acp, ...)`. `start()` now returns a `FirelineAgent`; this should be `agent.connect('code-review-agent')`.
- Medium: `examples/code-review-agent/index.ts:4,25` hides the state-observation story behind `waitForRows()` from `examples/shared/wait.ts:1-20`. This example should show direct collection subscription or direct collection inspection, not a generic helper wrapper.
- Low: `examples/code-review-agent/README.md:17-21` still shows `sandbox({ ..., envVars })`. Current idiom is `secretsProxy()` for secrets, not passing credentials through sandbox env vars.

## crash-proof-agent

- High: `examples/crash-proof-agent/index.ts:14` destroys the sandbox through `new SandboxAdmin({ serverUrl: primaryUrl }).destroy(first.id)`. The example already has `first: FirelineAgent`; this should be `first.stop()` or `first.destroy()`.
- Medium: `examples/crash-proof-agent/index.ts:10-18` uses `connectAcp(first.acp, ...)` and `connectAcp(second.acp, ...)` instead of `first.connect(...)` and `second.connect(...)`.
- Medium: `examples/crash-proof-agent/index.ts:4,20` still relies on `waitForRows()` instead of showing direct stream subscription.
- Low: `examples/crash-proof-agent/README.md:17-20` documents the same stale `SandboxAdmin.destroy(first.id)` pattern.

## approval-workflow

- High: `examples/approval-workflow/index.ts:1,10` imports and uses `appendApprovalResolved()` even though this example owns the launched agent and the approval resolver lives in the same process. Current idiom is `handle.resolvePermission(sessionId, requestId, { allow, resolvedBy })`; `appendApprovalResolved()` is the escape hatch for external contexts.
- Medium: `examples/approval-workflow/index.ts:23-26` still treats the `start()` result as a data record and calls `connectAcp(handle.acp, ...)` instead of `handle.connect('approval-workflow')`.
- Medium: `examples/approval-workflow/index.ts:4,29` uses `waitForRows()` rather than direct subscription to `db.permissions`.
- Low: `examples/approval-workflow/README.md:17-21` still shows `sandbox({ envVars })`, which teaches the wrong secret-management path.

## background-task

- Medium: `examples/background-task/index.ts:19-25` provisions correctly, but then calls `connectAcp(handle.acp, 'background-task')`. This should be `handle.connect('background-task')` so the example actually demonstrates the live-object API.
- Low: `examples/background-task/README.md:15-17` still shows `sandbox({ envVars })` instead of the current `secretsProxy()` pattern for secrets.

## live-monitoring

- No code findings. `examples/live-monitoring/index.ts:12-52` is the cleanest example in this set: it uses `fireline.db()`, flattened collections (`db.sessions`, `db.promptTurns`, `db.permissions`, `db.chunks`), `useLiveQuery`, and `useAcpClient`.
- Low: `examples/live-monitoring/README.md:15-19` is stale. It still documents `db.collections.sessions`, `db.collections.promptTurns`, `db.collections.permissions`, and `db.collections.chunks`, but the current API is the flattened `db.sessions`, `db.promptTurns`, `db.permissions`, `db.chunks`.

## multi-agent-team

- Medium: `examples/multi-agent-team/index.ts:10-13` gets back named `FirelineAgent` objects from `pipe(...).start()`, then immediately bypasses them with `connectAcp(handles.researcher.acp, ...)` and `connectAcp(handles.writer.acp, ...)`. These should be `handles.researcher.connect(...)` and `handles.writer.connect(...)`.
- Medium: `examples/multi-agent-team/index.ts:4,16,20` still uses `waitForRows()` instead of showing direct durable-stream subscription or collection reads.

## cross-host-discovery

- Medium: `examples/cross-host-discovery/index.ts:9-18` provisions two `FirelineAgent`s, then uses `connectAcp(agentB.acp, 'cross-host-discovery')`. This should be `agentB.connect('cross-host-discovery')`.
- Low: `examples/cross-host-discovery/index.ts:32-43` uses a custom `observeSessionText()` promise wrapper. This is at least stream-driven, not polling, but it still obscures the core observation pattern compared with directly subscribing to `db.chunks` / `db.promptTurns`.

## flamecast-client

- High: `examples/flamecast-client/ui/fireline-client.ts:62-145` and `:147-340` define a large custom `FlamecastClient` that reimplements session, runtime, filesystem, and queue APIs over bespoke REST calls. That is the opposite of the current Fireline split:
  control plane via `SandboxAdmin` or `FirelineAgent.stop()/destroy()`,
  session plane via ACP / `useAcpClient`,
  observation plane via `fireline.db()` + live collections.
- High: `examples/flamecast-client/ui/hooks/use-session-state.ts:84-126` still routes core session actions through that custom REST client: `client.resolvePermission(...)`, `client.terminateSession(...)`, `client.fetchSessionFileSystem(...)`, and `client.fetchSessionFilePreview(...)`. In the current idiom those belong on the ACP/live-agent path: `acp.resolvePermission(...)`, ACP file reads, and admin or `FirelineAgent.destroy()`.
- High: `examples/flamecast-client/ui/hooks/use-sessions.ts:4-10`, `ui/hooks/use-session.ts:4-10`, and `ui/hooks/use-runtimes.ts:4-10` poll REST endpoints through React Query. These should be live queries over the state DB, not `fetchSessions()`, `fetchSession()`, and `fetchRuntimes()` wrappers.
- High: `examples/flamecast-client/server.ts:7-20` imports directly from `../../packages/client/src/...` and `../../packages/state/src/...`. Examples should consume package exports, not raw source paths.
- High: `examples/flamecast-client/server.ts:148-218` and `:431-510` rebuild a custom `/api/*` layer for sessions, approvals, and runtimes even though Fireline already provides those planes. This makes the example teach custom infrastructure instead of Fireline.
- Medium: `examples/flamecast-client/server.ts:454-455` uses `connectAcp(handle.acp, ...)` immediately after `start()`. That should be `handle.connect(...)`.
- Medium: `examples/flamecast-client/server.ts:503-509` and `:630-636` use raw `appendApprovalResolved(...)` despite having the agent/session objects in-process. That should resolve through the live agent path where possible.
- Medium: `examples/flamecast-client/server.ts:442-445` and `:649-656` still push secrets through `envVars` and thread an untyped `provider: string` into `sandbox({ provider: options.provider, ... })`. Current idiom is `secretsProxy()` for secrets and typed provider selection, e.g. `sandbox({ provider: 'docker', image: '...' })`.
- Medium: `examples/flamecast-client/tsconfig.json:25-29` aliases `@fireline/client` and `@fireline/state` to raw workspace source paths. That hides packaging errors and reinforces non-package consumption. The same file still includes `shared/**/*.ts` at `:35` even though this browser example should not depend on custom shared ACP/approval helpers.

## Bottom line

The clean reference examples today are `live-monitoring` and, with one small fix, `background-task` and `cross-host-discovery`. The weakest surfaces are `approval-workflow`, `crash-proof-agent`, and especially `flamecast-client`, which still teach pre-live-object and pre-stream-first patterns.
