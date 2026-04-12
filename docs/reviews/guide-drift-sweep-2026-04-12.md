# Guide Drift Sweep — 2026-04-12

Compared `docs/guide/` against the current exported surfaces in:

- `packages/client/src/index.ts`
- `packages/client/src/agent.ts`
- `packages/client/src/db.ts`
- `packages/client/src/types.ts`
- `packages/state/src/schema.ts`
- `packages/state/src/index.ts`
- `packages/fireline/src/cli.ts`

This is a catalog only. No guide pages were changed in this pass.

## `docs/guide/README.md`

No drift found.

## `docs/guide/concepts.md`

- `High` `[docs/guide/concepts.md:30-58]` says `start()` does not return a live `FirelineAgent` and instead returns a `HarnessHandle` plus separate ACP/admin/state surfaces. Current `@fireline/client` returns `Promise<FirelineAgent>` from `Harness.start(...)`, and that object now owns `connect()`, `resolvePermission()`, `stop()`, and `destroy()`.
- `High` `[docs/guide/concepts.md:68-73]` says ACP is still a third-party concern and destructive lifecycle is still `SandboxAdmin`. That is stale against the current `FirelineAgent` surface.
- `High` `[docs/guide/concepts.md:98]` says Fireline does not wrap ACP at the package root. Current root exports include `connectAcp`.
- `Medium` `[docs/guide/concepts.md:104-108]` foregrounds `createFirelineDB(...)` and `db.collections.*` as the observation entrypoint. That still works, but current client-facing guidance should prefer `fireline.db()` / `db()` from `@fireline/client`.

## `docs/guide/compose-and-start.md`

No drift found.

## `docs/guide/middleware.md`

No drift found.

## `docs/guide/observation.md`

No drift found. The page still matches the currently exported collection names in `packages/state/src/schema.ts`, including `promptTurns`, `permissions`, `sessions`, `chunks`, and `runtimeInstances`.

## `docs/guide/approvals.md`

No drift found. The page correctly treats `FirelineAgent.resolvePermission(...)` as the primary same-process path and `appendApprovalResolved(...)` as the out-of-process escape hatch.

## `docs/guide/providers.md`

No drift found. The documented provider discriminated union matches `packages/client/src/types.ts`, and the microsandbox caveat still matches `crates/fireline-sandbox/src/provider_dispatcher.rs` on current main.

## `docs/guide/multi-agent.md`

- `High` `[docs/guide/multi-agent.md:30-35]` says `peer(...).start()` returns a name-keyed object of `HarnessHandle`s. Current implementation returns a name-keyed object of `FirelineAgent`s.
- `High` `[docs/guide/multi-agent.md:61-66]` says `fanout(...).start()` returns an array of generic handles. Current implementation returns `FirelineAgent[]`.
- `High` `[docs/guide/multi-agent.md:84-89]` says `pipe(...).start()` returns a name-keyed object of handles. Current implementation returns a name-keyed object of `FirelineAgent`s.
- `Medium` `[docs/guide/multi-agent.md:101-108]` still frames topology helpers as returning “handles” generically. That language is now underspecified for the shipped API surface.

## `docs/guide/resources.md`

No drift found.

## `docs/guide/cli.md`

- `High` `[docs/guide/cli.md:7-12]` lists the shipped CLI verb surface as `run`, `build`, and `agents`. Current CLI also ships `deploy`.
- `High` `[docs/guide/cli.md:104-105]` says deployment remains a later CLI phase. Current CLI ships `fireline deploy`.
- `High` `[docs/guide/cli.md:197]` says `fireline deploy --to <platform>` is deferred to later phases. That is now false.
- `Medium` `[docs/guide/cli.md:29-30]` shows `build --target` as the deploy-oriented path but does not show the current `deploy` command in the usage block beside it.
- `Medium` `[docs/guide/cli.md:152-160]` omits the current `deploy` flag table entirely and leaves `build` as the only documented target-selection flow.
- `Medium` `[docs/guide/cli.md:156]` lists scaffold outputs as `wrangler.toml`, `fly.toml`, `Dockerfile`, or `k8s.yaml`, but current `build` targets also include `docker-compose`.

## Phase 3 vocabulary note

No user-facing guide page currently drifts on the not-yet-finalized Phase 3 rename surface. The guides still describe the current shipped TS collection names from `packages/state/src/schema.ts`, so terms like `promptTurns` should stay untouched until the Phase 6 TS schema migration lands.
