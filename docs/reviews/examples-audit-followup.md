# Examples Audit Follow-up

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

Reference surfaces:
- `packages/client/src/index.ts`
- `packages/client/src/agent.ts`
- `packages/client/src/db.ts`
- `packages/client/src/middleware.ts`
- prior audit: `docs/reviews/examples-audit.md`

Global findings:
- The `connectAcp(handle.acp, ...)` issue is effectively gone from the simple examples. Remaining direct `connectAcp(...)` usage is concentrated in `flamecast-client/server.ts`.
- `db.collections.*` is gone from the audited example code and from `live-monitoring`’s README.
- `secretsProxy()` is now reflected correctly in the fixed READMEs.
- No audited example imports the deleted `shared/acp-node` or `shared/resolve-approval` helpers anymore.
- The main remaining anti-pattern in the simple examples is ad hoc `Promise + subscribe + timeout` waiting logic. It is better than polling, but several examples still reinvent a small `waitForRows()` inline.
- Raw source-path imports from `../../packages/client/src/...` remain only in `flamecast-client`.

Scoring:
- `10/10` = idiomatic current-main usage
- `7-9/10` = mostly idiomatic, one notable drift
- `4-6/10` = mixed, several outdated patterns
- `0-3/10` = fighting the library surface

## Summary

| Example | Old score | New score | Delta | Biggest remaining issue |
|---|---:|---:|---:|---|
| `code-review-agent` | 7/10 | 8/10 | +1 | The direct-subscribe replacement is still an inline wait helper |
| `crash-proof-agent` | 4/10 | 8/10 | +4 | Still uses a custom `waitForCompletedTurns()` helper instead of a cleaner subscription pattern |
| `approval-workflow` | 3/10 | 8/10 | +5 | Still wraps final-state waiting in `waitForResolvedPermissions()` |
| `background-task` | 8/10 | 10/10 | +2 | No substantive idiom issues found |
| `live-monitoring` | 9/10 | 10/10 | +1 | No substantive idiom issues found |
| `multi-agent-team` | 5/10 | 7/10 | +2 | Still relies on a custom `waitForCompletedTurns()` helper |
| `cross-host-discovery` | 8/10 | 9/10 | +1 | Still uses a custom `observeSessionText()` promise wrapper |
| `flamecast-client` | 2/10 | 4/10 | +2 | Still rebuilds a bespoke REST/session layer instead of leaning on Fireline’s three planes |

## code-review-agent

Improved:
- `examples/code-review-agent/index.ts:21` now uses `handle.connect('code-review-agent')` instead of bypassing the `FirelineAgent`.
- `examples/code-review-agent/README.md:17-27` now documents `secretsProxy(...)` instead of `envVars`.

Still suboptimal:
- `examples/code-review-agent/index.ts:24-46` replaced `waitForRows()` with a local `Promise` built around `db.permissions.subscribe(...)`, timeout management, and manual unsubscribe. This is stream-driven, but it is still a mini wait helper rewritten inline rather than a clean reactive example.

Verdict:
- Mostly idiomatic now. The remaining issue is presentation quality, not the core API surface.

## crash-proof-agent

Improved:
- `examples/crash-proof-agent/index.ts:9,15` now uses `first.connect(...)` and `second.connect(...)`.
- `examples/crash-proof-agent/index.ts:12` now uses `first.stop()` instead of `SandboxAdmin.destroy(...)`.
- `examples/crash-proof-agent/README.md:17-21` now documents `first.stop()` and `second.connect(...)`.

Still suboptimal:
- `examples/crash-proof-agent/index.ts:22-60` still uses a custom `waitForCompletedTurns()` helper. It is subscribe-based, not polling, but it still obscures the core “subscribe to the stream, react to state” pattern behind another local abstraction.

Verdict:
- The live-object migration landed correctly. What remains is cleanup of helper-shaped observation code.

## approval-workflow

Improved:
- `examples/approval-workflow/index.ts:21` now resolves through `handle.resolvePermission(...)` instead of raw `appendApprovalResolved(...)`.
- `examples/approval-workflow/index.ts:25` now uses `handle.connect('approval-workflow')`.
- `examples/approval-workflow/index.ts:24` now directly subscribes to `db.permissions` for approval dispatch.
- `examples/approval-workflow/README.md:17-25` now documents `secretsProxy(...)`.

Still suboptimal:
- `examples/approval-workflow/index.ts:36-70` still uses a local `waitForResolvedPermissions()` helper to gate the final printout. This is much better than the old `waitForRows()` import, but it is still the same helper pattern inlined locally.
- `examples/approval-workflow/README.md:11` says “appended back into the same state stream.” That remains true, but the code path is now `handle.resolvePermission(...)`; the wording could be updated to emphasize the live-agent method rather than the low-level event append.

Verdict:
- This example is now idiomatic enough to copy. The only remaining issue is the helper-style wait wrapper used for demo output.

## background-task

Improved:
- `examples/background-task/index.ts:24` now uses `handle.connect('background-task')`.
- `examples/background-task/README.md:16-25` now documents `secretsProxy(...)`.

Still suboptimal:
- No substantive usage issues found. `examples/background-task/index.ts:4-8,19-28` cleanly demonstrates `fireline.db(...)` for later observation and `FirelineAgent.connect(...)` for the active run.

Verdict:
- Idiomatic. This is now one of the clean reference examples.

## live-monitoring

Improved:
- `examples/live-monitoring/README.md:16-19` now matches the real flattened DB API: `db.sessions`, `db.promptTurns`, `db.permissions`, `db.chunks`.

Still suboptimal:
- No substantive usage issues found. `examples/live-monitoring/index.ts:40-52` remains the cleanest observation example in the tree.

Verdict:
- Idiomatic. This is the best reference example for the observation plane.

## multi-agent-team

Improved:
- `examples/multi-agent-team/index.ts:11-12` now uses `handles.researcher.connect(...)` and `handles.writer.connect(...)`.

Still suboptimal:
- `examples/multi-agent-team/index.ts:23-55` still uses a local `waitForCompletedTurns()` helper. Like the other inline wait rewrites, it is not polling, but it still hides the stream-first observation story behind a one-off utility.
- `examples/multi-agent-team/index.ts:15-20` still leans on “wait for completion, then scrape arrays” rather than a clearer subscription/query-builder story.

Verdict:
- Improved, but not yet a clean reference example. The topology API usage is good; the observation code is still a bit ad hoc.

## cross-host-discovery

Improved:
- `examples/cross-host-discovery/index.ts:14` now uses `agentB.connect('cross-host-discovery')`.

Still suboptimal:
- `examples/cross-host-discovery/index.ts:32-44` still uses a custom `observeSessionText()` wrapper. It is stream-driven and small, so this is less severe than the other helpers, but it still packages the observation logic into a one-off promise instead of showing the raw collection subscription/query pattern.

Verdict:
- Very close to idiomatic. The main story is correct; only the observation helper remains slightly stylized.

## flamecast-client

Improved:
- `examples/flamecast-client/ui/fireline-client.ts:1-2` now imports from package exports, not raw source paths.
- `examples/flamecast-client/ui/provider.tsx:2` now uses `db` from `@fireline/client`.
- `examples/flamecast-client/ui/hooks/use-session-state.ts:5-12,29-49,51-59` now uses `@fireline/state` query builders plus `useAcpClient`, which is materially closer to the intended architecture.
- The deleted helper imports are gone.
- `examples/flamecast-client/tsconfig.json:32-39` no longer includes `shared/**/*.ts`.

Still suboptimal:
- `examples/flamecast-client/server.ts:7-20` still imports from `../../packages/client/src/...` and `../../packages/state/src/...`. This violates the package-consumption rule.
- `examples/flamecast-client/tsconfig.json:25-29` still aliases `@fireline/client` and `@fireline/state` to raw workspace source files, which masks packaging issues and reinforces non-package usage.
- `examples/flamecast-client/ui/fireline-client.ts:62-145,147-260` still defines a broad custom `FlamecastClient` surface with REST methods like `fetchSessions()`, `fetchSession()`, `fetchRuntimeFileSystem()`, and `terminateSession()`. That keeps the old custom-infrastructure shape alive.
- `examples/flamecast-client/ui/hooks/use-sessions.ts:15-19`, `ui/hooks/use-session.ts:17-22`, and `ui/hooks/use-runtimes.ts:4-10` still depend on bespoke REST metadata fetches. `useRuntimes()` is still purely polling.
- `examples/flamecast-client/ui/hooks/use-session-state.ts:130-144` still uses `appendApprovalResolved(...)` even though the hook already has `useAcpClient(...)`. That should resolve through the ACP/session layer, not the raw event append helper.
- `examples/flamecast-client/ui/hooks/use-session-state.ts:166-181` still uses REST-style `terminateSession`, `fetchSessionFileSystem`, and `fetchSessionFilePreview` methods rather than the live-session / ACP path.
- `examples/flamecast-client/server.ts:456` still calls `connectAcp(handle.acp, ...)` immediately after `start()`.
- `examples/flamecast-client/server.ts:491-496` still destroys sandboxes through `admin.destroy(...)` instead of the live agent object when it has one.
- `examples/flamecast-client/server.ts:499-512` and `:632-638` still use `appendApprovalResolved(...)` directly.
- `examples/flamecast-client/server.ts:443-446,642-656` still pushes secrets through `envVars` and threads provider choice through a server-owned `ProviderName` path instead of demonstrating `secretsProxy()` and a cleaner declarative provider config.

New issues introduced by the migration:
- The UI is now hybrid in a slightly confusing way: some hooks are stream-native (`use-session-state`), while others still merge stream rows with REST metadata (`use-sessions`, `use-session`) or remain purely REST (`use-runtimes`). That is better than the old state, but less coherent than a full move to Fireline-native planes.

Verdict:
- Better than before, but still not demo-grade. The migration improved the edges, not the architecture.

## Final verdict

Now idiomatic:
- `background-task`
- `live-monitoring`

Mostly idiomatic, minor cleanup left:
- `code-review-agent`
- `crash-proof-agent`
- `approval-workflow`
- `cross-host-discovery`

Improved but still needs meaningful cleanup:
- `multi-agent-team`

Still architecturally non-idiomatic:
- `flamecast-client`

Bottom line:
- The major live-object fixes landed successfully.
- The remaining debt in the simple examples is mostly cosmetic/helper-shaped.
- The remaining debt in `flamecast-client` is still structural: it continues to rebuild custom control/session/observation layers instead of fully embracing Fireline’s three-plane model.
