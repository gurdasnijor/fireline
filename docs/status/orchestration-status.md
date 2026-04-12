# Fireline Orchestration Status

> Live coordination doc for parallel Opus orchestrators + their dispatched agents.
>
> Updated: 2026-04-12

## How to use this doc

If you are an Opus agent coming in fresh, read this doc FIRST before dispatching anything. Then read the four canonical-identifiers docs (§Core proposals below) to understand the architectural direction.

Each orchestrator owns specific workstreams. Do NOT dispatch into workspaces owned by the other orchestrator without explicit handoff.

## Ownership split

| Orchestrator | Owns workstreams | Owns workspaces (cmux) |
|---|---|---|
| **Opus 1** (original session) | Canonical-identifiers execution chain + verification + TLA | w12, w13, w17 |
| **Opus 2** (new session) | Examples, demos, docs, proposal drift fixes, CI, deployment story | w15, w18, w19 (+ any new codex sessions created by the user for Opus 2's streams) |
| Either | Emergency intervention on any workspace | — |

Shared: commit + push operations on `main`. Each orchestrator commits their own dispatches' outputs; do not step on each other's working tree changes if running in the same clone.

## Core proposals (read first if you're new)

1. `docs/proposals/acp-canonical-identifiers.md` — **the governing acceptance criterion.** No synthetic ids on agent-layer rows; ACP schema types only; plane separation; W3C Trace Context via `_meta`.
2. `docs/proposals/acp-canonical-identifiers-execution.md` — 8-phase execution plan with gates.
3. `docs/proposals/acp-canonical-identifiers-verification.md` — 5-layer verification (TLA+, Stateright, cargo audit, migration fixtures, E2E scenarios).
4. `docs/proposals/durable-subscriber.md` — the DurableSubscriber primitive (framework-level).
5. `docs/proposals/durable-promises.md` — imperative companion (user-level, `ctx.awakeable()`).
6. `docs/reviews/approval-gate-correctness.md` — proven semantic substrate the refactor builds on.

## Current in-flight state (as of this doc's date)

| Workspace | Owner | Task | ETA signal |
|---|---|---|---|
| w12 | Opus 1 | canonical-ids Phase 0 + Phase 1 (type layer) | In progress |
| w13 | Opus 1 | verification/audit crate + forbidden-identifier grep | In progress |
| w17 | Opus 1 | TLA+ spec extensions + TLC run on small model | In progress |
| w15 | Opus 2 | proposal consistency audit → `docs/proposals/proposal-index.md` | In progress |
| w18 | Opus 2 | examples cleanup per docs/reviews/examples-audit-followup.md | In progress |
| w19 | Opus 2 | (to be dispatched) CI test harness fix (`/v1/runtimes` → `/v1/sandboxes`) | Pending dispatch |

## Sequencing rules (non-negotiable)

1. **Phase N+1 of canonical-identifiers execution cannot start until Phase N's verification gate is green.** See the execution doc's gate table.
2. **DurableSubscriber implementation cannot start until canonical-identifiers Phases 2 + 5 have landed** (Phase 2: approval gate uses canonical RequestId; Phase 5: W3C Trace Context propagation).
3. **Audit tooling (`verification/audit/`) runs in warn mode until Phase 1.5 lands.** After Phase 1.5, it flips to strict mode as a CI gate.
4. **Proposal drift fixes (from `proposal-index.md`) can happen in parallel with Phase execution — they don't block or get blocked.**

## Workstream dispatch queues

### Canonical-identifiers chain (Opus 1)

After w12 (Phase 0+1) lands:
- Phase 1.5 — replace String ACP id fields with sacp::schema types in agent-layer rows. Dispatch to a fresh workspace.
- Phase 2 — approval gate uses canonical JSON-RPC RequestId (replaces the SHA256 hash derivation).
- Phase 3 — StateProjector canonical rekeying.
- Phase 4 — W3C Trace Context propagation.
- Phase 5 — delete ActiveTurnIndex + child_session_edge.
- Phase 6 — TS schema migration.
- Phase 7 — plane separation enforcement (drop host_key/host_id from SessionRecord).
- Phase 8 — cleanup + migration scaffolding removal.

### Examples / demos / docs (Opus 2)

After w15's proposal-index lands:
- Dispatch drift fixes per the index's priority ordering (critical > design > naming > historical).
- Revise `docs/demos/pi-acp-to-openclaw.md` to reflect canonical-ids + durable-promises once those primitives exist.
- Once DurableSubscriber lands in code (post-canonical-ids Phase 5): rewrite `examples/approval-workflow/` to use a subscriber rather than inline subscribe.
- Once awakeables ship: add an example demonstrating `ctx.awakeable()`.

### CI (Opus 2)

- w19 dispatched: fix `/v1/runtimes` → `/v1/sandboxes` in remaining test files (tests/control_plane_docker.rs, tests/control_plane_push.rs, etc.). Infrastructure-layer work, does not conflict with canonical-identifiers refactor.
- Post-Phase 1.5: re-run audit tooling, validate zero violations in already-converted crates.

## Coordination conventions

- **Commit + push cadence:** each orchestrator commits their dispatched work's output as soon as it lands AND builds clean. Do not batch unrelated dispatches into one commit.
- **Commit messages:** reference which proposal/phase the work implements. Format: `{Phase/proposal slug}: {short summary}`.
- **If a workspace becomes idle,** the owning orchestrator dispatches the next task in their queue. Don't hand it across orchestrators — just dispatch fresh.
- **If a workspace crashes / session exits,** any orchestrator can recycle it by re-dispatching. Note the recycle in this doc.
- **Status updates:** update this doc when a workspace changes ownership, when a phase gate passes, or when a new sequencing rule emerges.

## Handoff protocol for new Opus

Opus 2, on first boot:

1. Read this doc.
2. Read the four core canonical-identifiers proposals.
3. Check `git log --oneline -20` to see recent commits.
4. Run `cmux list-workspaces --window window:1` to confirm the current topology.
5. Spot-check your owned workspaces (w15, w18, w19) via `cmux read-screen --workspace workspace:N --lines 10`.
6. If any are idle, dispatch the next task from your queue above.
7. Confirm with Opus 1 (via the user, or a shared note in this doc) before making architectural decisions that cross workstream boundaries.

## Open architectural decisions (cross-orchestrator)

- Whether `webhook-support.md` merges into `durable-subscriber.md` or stays standalone — waiting on w15's proposal-index audit to recommend.
- Whether the `fireline.db()` session-flattened DB should be restructured in Phase 6 or deferred to a post-refactor proposal — pending Phase 6 execution reality.
- Whether the "always-on" sandbox policy (from the deployment proposal) ships with DurableSubscriber or as its own execution phase — pending DurableSubscriber implementation.

## Known risks

1. Coordination drift if this doc isn't kept current.
2. The canonical-identifiers refactor touches many files; merge conflicts between Opus 1 and Opus 2 dispatches are possible if Opus 2 agents edit agent-layer code during Phase execution. Opus 2 dispatches should generally NOT edit agent-layer source during canonical-ids Phases 1.5–7.
3. If TLC finds a spec bug in w17's work, the refactor phase-gate invariants may need adjustment — propagate to the execution plan.
