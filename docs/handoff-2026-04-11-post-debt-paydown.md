# Session Handoff — Post-Debt-Paydown State

> **Created:** 2026-04-11
> **Author:** Claude Opus 4.6 session, coordinating with workspace:4
> **For:** the next Claude Code session continuing this work
> **Context usage at handoff:** ~27%
> **Focus:** post-debt-paydown repo state, demo readiness, and remaining substrate gaps

---

## Your role

You are picking up Fireline after a large debt-paydown push on `main`. The immediate job is not broad exploration. It is to:

1. Preserve the newly-tightened architectural boundaries.
2. Avoid regressing the now-honest managed-agent docs and scoreboard.
3. Push the remaining substrate work only where it is still real:
   - TS `resume(sessionId)` helper
   - Slice 17 live dispatch for `TransportRef::McpUrl`
   - runtime-host split / cleanup proposal follow-through
4. Keep commits small, reviewable, and attribution-clean.

## TL;DR — repo state right now

```
HEAD: 6045c4a Unify control-plane liveness ownership
```

- `src/lib.rs` public surface is intentionally small: `bootstrap`, `control_plane_client`, `orchestration`, `runtime_host`, `runtime_registry`, `stream_host`
- `bootstrap::AppState` is `pub(crate)` and binary-internal glue has been pushed back behind the public surface
- control-plane liveness ownership is now unified under `RuntimeRegistry`
- the managed-agent status docs are honest:
  - `docs/explorations/managed-agents-mapping.md`
  - `docs/explorations/managed-agents-citations.md`
- the semantic kernel / verification lane has landed:
  - `crates/fireline-semantics`
  - `verification/`

## Working tree at handoff

Clean.

```text
git status --short
# no output
```

That means the next session starts from a clean `main`, not from an in-flight swarm merge state.

## Session summary

This debt-paydown push materially improved the repo shape:

- public-vs-private crate boundaries were tightened
- control-plane liveness ownership was simplified
- managed-agent docs were brought back in sync with the real test suite
- the harness durable suspend/resume gap was closed
- semantics / verification infrastructure now exists in-tree

The result is that the repo is in a better state both for demo-readiness and for architectural reasoning. The remaining work is narrower and more obviously downstream of the current substrate, rather than "unknown missing primitives."

## Commits in this phase

Snapshot of the key landed commits, newest first, at handoff time:

```text
6045c4a Unify control-plane liveness ownership
dcf93bf Fix review findings from 8301c61 and 0a30269
b82f642 Add fireline-semantics crate and verification spec with Stateright checks
d3d7e7f Replace architecture doc with comprehensive reference from arch2 synthesis
0a30269 Scope down AppState and tighten the last three lib.rs modules
4eaf94a Add resources field and ResourceRef type to @fireline/client host API
8301c61 Tighten src/lib.rs public surface
02363e6 Align managed-agents citations doc with current live coverage
a4dc19c Promote harness durable suspend/resume round-trip via cross-runtime client shim
cbf49a1 Align managed-agents mapping doc with live test coverage
5729584 Clarify Docker-scoped managed-agent resource tests as cross-reference markers
f62405e Remove outdated product documentation files
8c36708 Fix managed-agent workflow for ignored Cargo.lock
3dc7d3a Add managed-agent suite GitHub Actions workflow
```

If you are reading this later than the moment it was written, run `git log --oneline -10` first. There may be one or more additional commits beyond this snapshot.

## Architectural state now

### 1. `src/lib.rs` has a deliberately small public surface

The public surface is now exactly six modules:

- `bootstrap`
- `control_plane_client`
- `orchestration`
- `runtime_host`
- `runtime_registry`
- `stream_host`

Everything else is crate-private process glue. This is intentional. If a future test or binary wants a crate-private type, the right move is to promote a narrow API, not to broaden visibility casually.

### 2. `AppState` is no longer accidental API

`bootstrap::AppState` is now `pub(crate)`. That is the right direction. It prevents the request-routing/materializer glue from becoming an external substrate promise by accident.

### 3. Liveness ownership is unified under `RuntimeRegistry`

The control-plane liveness story is less split now. The new state should be read as:

- `RuntimeRegistry` is the owning authority for liveness bookkeeping
- avoid reintroducing duplicated "last seen" or "staleness" sources of truth in sidecar layers

One important semantic subtlety from this session: a proposed persisted `last-seen` approach was intentionally kept in-memory to preserve restart semantics. Do not casually "durabilize" liveness timestamps without re-evaluating what restart is supposed to mean.

### 4. Managed-agent docs are finally honest

Two docs now match the actual suite rather than stale intuition:

- `docs/explorations/managed-agents-mapping.md`
- `docs/explorations/managed-agents-citations.md`

Do not let these drift again. If a test is promoted, ignored, or reframed as a Docker-scoped cross-reference marker, update the docs in the same lane or immediately after.

### 5. Semantic kernel and verification now exist

`b82f642` landed:

- `crates/fireline-semantics`
- `verification/` with Stateright checks

Treat these as real new architectural assets, not as throwaway experiment folders.

### 6. Architecture docs were rewritten, not merely patched

`d3d7e7f` replaced the architecture doc from the `arch2` synthesis. The repo now has a more coherent written architectural reference than it had at the start of the day. Read that before proposing a broad structural change.

## Demo-readiness scoreboard

**Managed-agent demo scoreboard: 28 live managed-agent contracts.**

This count intentionally focuses on demo-facing managed-agent coverage, not every meta/component assertion in the suite.

Breakdown:

- **Session:** 5 live
  - append-only replay
  - durable across runtime death
  - replay from captured offset
  - idempotent append under retry
  - materialized-vs-raw agreement
- **Sandbox:** 3 live
  - reachable provision
  - configured-once / many-executes
  - stop + recreate preserves `session/load`
  - plus one Docker-scoped cross-provider marker delegated elsewhere
- **Harness:** 5 live
  - every effect logged
  - append-order stable
  - approval-gate blocks until resolved
  - durable suspend/resume round trip
  - seven-combinator coverage
- **Orchestration:** 4 live
  - cold-start acceptance contract
  - resume-on-live-runtime is no-op
  - concurrent resume converges to one runtime
  - subscriber loop drives pause-release
- **Resources:** 6 live
  - local path mounter
  - local file backend route-through
  - fs backend captures ACP write as durable event
  - stream-backed cross-runtime reads
  - primitives-suite physical-mount acceptance sibling
  - primitives-suite fs-backend acceptance sibling
  - plus one Docker-scoped shell-visible-mount marker delegated elsewhere
- **Tools:** 4 live
  - schema-only descriptor contract
  - transport-agnostic registration
  - first-attach-wins collision rule
  - primitives-suite schema-only acceptance sibling
- **Baseline managed-agent smoke:** 1 live

Two useful clarifications:

- Raw Cargo counts will be slightly higher than `28` because the primitives suite also contains meta/component checks such as the inventory test and fs-backend component test.
- The D promotion is already on `main`: `a4dc19c` made `harness_durable_suspend_resume_round_trip` live.

## Remaining architectural debt

### 1. RuntimeHost split is still not concretized

Workspace:10 was writing a proposal doc at:

```text
docs/proposals/runtime-host-split.md
```

At this handoff snapshot, that file is **not present / not committed**.

That means the design conversation still exists, but the proposal artifact is not yet on `main`. If the next session takes this up:

1. Check whether workspace:10 landed something after this handoff.
2. If not, write/read the proposal before changing `RuntimeHost` structure directly.

### 2. TypeScript `resume(sessionId)` helper is still unbuilt

Rust-side orchestration is strong. The TS-side ergonomic surface is not there yet.

This is now a real remaining gap because:

- `fireline::orchestration::resume(...)` exists and is live
- the managed-agent docs now explicitly describe the missing TS ownership

The next TS lane should build the helper on top of the current explicit shared-state endpoint shape, not reintroduce any implicit discovery hack.

### 3. Slice 17 live dispatch for `TransportRef::McpUrl` is still unbuilt

Slice 17 landed the data model:

- `ToolDescriptor`
- `TransportRef`
- `CredentialRef`
- `CapabilityRef`

What did **not** land is the live dispatch path for remote MCP URL tools. Right now the descriptor and collision semantics are real, but remote tool invocation via `TransportRef::McpUrl` remains future work.

Do not misread slice 17 as "done done." It is done at the descriptor/attachment layer, not at the transport execution layer.

## Known gotchas from this session

### 1. Cargo file-lock contention between parallel agents caused real pain

Parallel agents doing `cargo build` / `cargo test` at the same time blocked each other on:

- package cache locks
- artifact directory locks

That produced slowdowns and, in earlier sweeps, SIGTERM-like failures from task timeouts. If you spawn a swarm again, serialize heavy Cargo work or at least stage it carefully.

### 2. Liveness persistence had a real semantic trap

Workspace:9 surfaced an important mismatch while unifying control-plane liveness:

- persisting "last seen" sounds cleaner
- but it changes restart semantics in subtle ways

The decision taken here was to keep that particular liveness notion in-memory so restart behavior remains honest. If you revisit this, do it as a semantics discussion, not as a "simple cleanup."

### 3. Review fixes around public surface uncovered re-export fragility

`dcf93bf` fixed review fallout from the visibility tightening. One concrete lesson: tightening `src/lib.rs` can expose hidden assumptions in tests and binaries about what the crate root re-exports or no longer exposes.

The concrete symptom called out during this phase was that `SessionIndex` access assumptions needed correction after the visibility cleanup. If you tighten the public surface further, expect this class of breakage again and audit rustdoc links / re-exports at the same time.

### 4. Known pre-existing flake: orchestration acceptance sometimes 500s on resume-create

This was already known before the doc lanes and is not caused by the debt-paydown doc work. If you see:

```text
managed_agent_orchestration_acceptance_contract
control plane rejected resume create
500 Internal Server Error
```

assume pre-existing orchestration flake first, not regression from the doc/visibility lanes.

## Recommended first steps for the next session

1. Run `git log --oneline -10` and `git status --short` before doing anything else.
2. Check whether `docs/proposals/runtime-host-split.md` landed after this handoff.
3. Pick exactly one of:
   - TS `resume(sessionId)` helper
   - runtime-host split proposal / implementation prep
   - Slice 17 live dispatch for `TransportRef::McpUrl`
4. Keep `managed-agents-mapping.md` and `managed-agents-citations.md` in sync with reality if you change any managed-agent contract status.

## Bottom line

This repo is past the "unknown shape debt" phase for the managed-agent substrate.

What is now true:

- docs are more honest
- crate boundaries are tighter
- liveness ownership is cleaner
- semantics/verification infrastructure exists
- managed-agent demo coverage is materially stronger

What is still left is narrower:

- a TS orchestration surface
- capability transport execution depth
- a clean RuntimeHost split story

That is a better next-session starting point than "find the missing primitive." The missing primitives are largely gone.
