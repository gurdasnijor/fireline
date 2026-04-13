# Demo Code Leakage Audit (2026-04-12)

Reviewed on `main` at `62ba5d3`.

Method:
- `git log --since="12 hours ago"` as the starting window, then extended by `git log -S ...` and `git blame` when a current production-path behavior traced to an older commit.
- `rg` sweeps across `packages/fireline/src`, `packages/client/src`, `crates/*/src`, `tests/*`, `packages/*/test`, `scripts/`, and `docker/` for demo-specific vocabulary and branching.
- Comparison against the advertised CLI surface in `docs/guide/cli.md`.

## Exec Summary

- `KEEP`: 4
- `MOVE`: 2
- `REVERT`: 1

The leakage is concentrated in the TypeScript CLI/client layer. I found **no Rust-side `demo mode` branching** in `crates/*/src` and no `spec.name` / env-var demo routing on the Rust path. The two highest-risk findings are:

1. `packages/client/src/sandbox.ts` overloads `approve({ scope: 'tool_calls' })` into a prompt-wide fallback gate. That is a public-semantic mismatch, not just a test convenience.
2. `packages/fireline/src/cli.ts` now performs hidden `pnpm --filter ... build` work for repo-local specs. That widens side effects beyond the advertised CLI contract and looks like a demo-era convenience shim.

## Findings

| SHA | File:line range | What changed | Class | Justification | Recommended target |
|---|---|---|---|---|---|
| `14adeb4` | `packages/fireline/src/resolve-binary.ts:41-87` | Release-first, debug-second binary lookup with explicit error text. | `KEEP` | This matches `docs/guide/cli.md:176-193` exactly and improves the real CLI contract. No demo names, spec-path special cases, or hidden side effects. | `N/A` |
| `14adeb4` | `packages/fireline/src/cli.ts:445-473,741-754` | Reuse healthy `fireline-streams`, refuse a healthy host port early, and log mixed release/debug profiles. | `KEEP` | This is operator-safe behavior now documented in `docs/guide/cli.md:84-101,185-187`. It is generic CLI hardening, not a demo-only branch. | `N/A` |
| `14adeb4` | `packages/fireline/src/cli.ts:560-599`; `packages/fireline/tsconfig.loader.json:1-13` | Load specs through `tsx` with a repo-aware tsconfig and unwrap nested default exports. | `KEEP` | The motivating case was a repo-root demo spec, but the implementation is generic: it fixes workspace alias resolution and `tsImport` wrapper handling for any repo-local spec. No hard-coded `docs/demos/...` logic exists in the runtime path. | `N/A` |
| `1d4f58b` | `packages/fireline/src/cli.ts:601-623` | `loadSpec()` reads the spec source, detects `@fireline/client` / `@fireline/state`, and runs `pnpm --filter ... build` on demand. | `REVERT` | This is hidden build orchestration in the public CLI path. It is **not** part of the advertised contract in `docs/guide/cli.md:21-24,176-193`, and it papers over repo/demo setup gaps instead of fixing the packaging story. | `N/A` |
| `14adeb4` | `packages/fireline/src/cli.test.ts:120-123` | Package unit test hard-codes `docs/demos/assets/agent.ts` as the loader fixture. | `MOVE` | The behavior worth preserving is “repo-root spec loads with workspace aliases,” not “the demo asset loads in package unit tests.” Demo-asset validation belongs in demo integration coverage; package tests should use a neutral fixture. | `docs/demos/scripts/` for demo validation; replace package-unit fixture with a generic spec under `packages/fireline/test-fixtures/` |
| `18e8db4` | `packages/client/src/sandbox.ts:237-252` | `approve({ scope: 'tool_calls' })` lowers to `match: { kind: 'promptContains', needle: '' }` with reason `approval fallback: prompt-level gate until tool-call interception lands`. | `MOVE` | The public contract in `packages/client/src/middleware.ts:32-46` and `packages/client/src/types.ts:127-140` advertises tool-call scoping. The current lowering gates every prompt. That fallback may be useful for demos/examples, but it should not live behind the production `tool_calls` surface. | `examples/approval-workflow/` or `docs/demos/assets/` as an explicit prompt-gate demo helper until true tool interception lands |
| `3e22489` | `docker/bin/fireline-embedded-spec-bootstrap.ts:158-174`; `docker/bin/fireline-host-quickstart-entrypoint.sh:60-69` | Forward `--advertised-state-stream-url` through embedded-spec quickstart boot. | `KEEP` | This change came from demo pressure, but the code is a correctness fix for the quickstart artifact, not a demo-mode branch. The quickstart surface is already explicitly namespaced under `docker/*quickstart*` and documented as non-production in `docker/README.md`. | `N/A` |

## No-Finding Sweeps

- `crates/*/src`
  - No hits for `cfg!(feature = "demo")`, demo env vars, `spec.name` demo matching, or OpenClaw/Telegram/Discord branching in production Rust sources.
  - Recent Rust/demo-driven commits in scope (`8947083`, `3e22489`) were correctness fixes, not demo modes.

- `tests/*`, `packages/*/test/*`
  - The only explicit demo-path expectation I found in a production test suite was `packages/fireline/src/cli.test.ts:120-123`.
  - Other hits like `pi-acp` in `agents add pi-acp` are contract-level registry examples, not demo-capture assertions.

- `scripts/`
  - No merged `demo-preflight`-style wrapper or equivalent landed.
  - The rejected wrapper attempt was correctly stood down before merge, so there is no script-level demo leakage to clean up from that lane.

- `docker/`
  - The quickstart image is explicitly quarantined as a quickstart/demo convenience image in `docker/README.md`.
  - I did **not** find demo-name branching in the Docker entrypoints themselves; the recent quickstart fix is data-path correctness, not a demo switch.

## Recommended Refactor PRs To Reopen

1. **CLI: remove hidden workspace package autobuild from `loadSpec()`**
   - Delete `ensureWorkspaceSpecDependenciesBuilt()` / `buildWorkspacePackage()`.
   - Replace with either:
     - a real packaged resolution contract, or
     - an explicit documented developer command outside the public `run` path.

2. **Client: stop lying about `approve({ scope: 'tool_calls' })`**
   - Either implement real tool-call interception, or
   - rename/extract the prompt-wide fallback into demo/example-only code and keep the public middleware semantics honest.

3. **CLI tests: replace demo-asset fixture with a generic repo-root fixture**
   - Keep one package-unit test for workspace alias resolution.
   - Move the literal `docs/demos/assets/agent.ts` assertion to demo replay/integration coverage.

## Architect Checklist

- Confirm that public CLI behavior may not hide `pnpm build` side effects for repo-local convenience.
- Confirm that public middleware semantics must match their type/docs contract, especially for `approve({ scope: 'tool_calls' })`.
- Confirm that demo walkthrough validation lives under `docs/demos/` or a dedicated integration lane, not package-unit suites.
- Confirm that the `docker/*quickstart*` artifact remains explicitly quarantined and does not drive the core package/API contract.
- Confirm whether adjacent root-workspace metadata changes (for example `538f0c4`) need a follow-up audit outside this code-path-only scope.
