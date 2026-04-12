# Fireline Orchestration Status

> Live coordination doc for three parallel Opus orchestrators + their dispatched agents.
>
> Updated: 2026-04-12

## Three-role model

| Role | Workspace | Responsibilities |
|---|---|---|
| **Opus 1 — Orchestrator** | original session (Manager 1) | Drive execution. Dispatch sequential work (canonical-ids Phase N → N+1). Commit + push outputs. Sequence the refactor. Technical implementation steering. |
| **Opus 2 — PM** | w10 (Manager 2) | Own docs, proposals, status artifacts. Track workstream completion against proposals. Flag drift between proposals and execution reality. Keep planning artifacts clean, consistent, dispatch-ready. Proactively coordinate — surface blockers, propose sequencing adjustments. |
| **Opus 3 — Architect** | w20 (Manager 3) | Own technical quality. Review all landed code for architectural alignment: durable-streams as source of truth, correct conductor usage, canonical ACP identifiers. Can draft high-level proposals. Make technical decisions when ambiguous. Deep ACP spec + conductor knowledge. |

### Division of labor

- **Opus 1** dispatches and sequences execution. Owns the operational cadence.
- **Opus 2** owns the PLANNING surface — proposals, index, status docs. Proactively reports drift.
- **Opus 3** owns the TECHNICAL surface — code review, proposal technical soundness, architectural decisions.

When ambiguity arises:
- Planning question (what order, what scope, is this ready to dispatch?) → Opus 2
- Architectural question (is this aligned with durable-streams-as-truth? is this ACP-compliant?) → Opus 3
- Dispatch question (who runs this task, when, where) → Opus 1

## Core references (read first if you're new)

### Proposals
1. `docs/proposals/acp-canonical-identifiers.md` — governing acceptance criterion.
2. `docs/proposals/acp-canonical-identifiers-execution.md` — 8-phase execution plan.
3. `docs/proposals/acp-canonical-identifiers-verification.md` — 5-layer verification.
4. `docs/proposals/durable-subscriber.md` — framework primitive.
5. `docs/proposals/durable-promises.md` — imperative companion.
6. `docs/reviews/approval-gate-correctness.md` — proven substrate.

### ACP / conductor
- ACP spec: https://agentclientprotocol.com/protocol/schema
- ACP extensibility (_meta): https://agentclientprotocol.com/protocol/extensibility#the-_meta-field
- Meta-propagation RFD: https://agentclientprotocol.com/rfds/meta-propagation#implementation-details
- Conductor architecture: https://agentclientprotocol.github.io/symposium-acp/conductor.html
- ACP Rust SDK schema: https://github.com/agentclientprotocol/rust-sdk/tree/main/src/agent-client-protocol-core/src/schema
- Durable streams: https://durablestreams.com/stream-db
- Durable streams server integration: https://thesampaton.github.io/durable-streams-rust-server/integration/sessions.html

## Current workspace topology

| Workspace | Owner / Role | Task |
|---|---|---|
| w10 | **Opus 2 (PM)** | onboarding |
| w20 | **Opus 3 (Architect)** | onboarding |
| w12 | Opus 1 → codex | canonical-ids Phase 0 + Phase 1 (type layer) |
| w13 | Opus 1 → codex | verification/audit crate |
| w17 | Opus 1 → codex | TLA+ spec extensions + TLC run |
| w15 | Opus 2 → codex | proposal consistency audit → proposal-index.md |
| w18 | Opus 2 → codex | examples cleanup per audit followup |
| w19 | Opus 2 → codex | CI test harness fix (/v1/runtimes → /v1/sandboxes) |
| w21 | **unassigned codex** | idle — Opus 2 or Opus 3 may claim |
| w22 | **unassigned codex** | idle — Opus 2 or Opus 3 may claim |

### Codex claiming protocol

When an Opus claims an unassigned codex, update this table with the new owner and task. When the codex completes, either recycle it for the next task in that Opus's queue or release it back to unassigned.

### Workspace ownership rules

- Codex agents belong to one Opus orchestrator at a time (noted in the table above).
- If a codex workspace goes idle, its OWNING Opus dispatches the next task.
- Do NOT dispatch into another Opus's workspace without explicit handoff.
- If you want to recycle an exited session, claim it explicitly in this doc first.

## Sequencing rules (non-negotiable)

1. Phase N+1 of canonical-ids execution cannot start until Phase N's verification gate is green.
2. DurableSubscriber implementation cannot start until canonical-ids Phases 2 + 5 land.
3. Audit tooling runs in warn mode until Phase 1.5 lands, then flips to strict.
4. Proposal drift fixes can happen in parallel with Phase execution.
5. **Architect (Opus 3) has veto power** on any landed code that violates core architectural primitives. If Opus 3 flags a regression, the landing is reverted until the architectural issue is resolved.

## Dispatch queues

### Opus 1 (execution chain)
- Monitor w12/w13/w17 → commit their work as it lands.
- After Phase 1 lands: dispatch Phase 1.5 (String → canonical ACP type field renames).
- After Phase 1.5: dispatch Phase 2 (approval gate uses canonical RequestId).
- Continue through Phase 8.
- Review Phase outputs against Opus 3's architectural gates before pushing.

### Opus 2 (PM)
- Monitor w15 → commit `proposal-index.md` when it lands. Dispatch follow-up drift fix patches per the index.
- Monitor w18 → commit examples cleanup.
- Monitor w19 → commit CI fix.
- Own this orchestration-status.md — keep it current.
- Watch for sequencing or scope drift across proposals. Propose adjustments to Opus 1.
- When a phase lands, update the "Current workspace topology" table and mark phase gates in the execution doc.

### Opus 3 (Architect)
- Review landed code after each phase for:
  - Durable-streams-as-truth invariants held
  - No new synthetic identifiers introduced in agent-layer code
  - ACP schema types used correctly
  - Conductor composition is idiomatic
  - `_meta` propagation is present on peer boundaries
- Can draft high-level proposals (new primitives, architectural direction) that Opus 1 or Opus 2 then operationalize.
- Can reject a phase landing if it violates architectural invariants.
- Owns the technical side of open questions listed below.

## Coordination conventions

- **Commit + push:** each orchestrator commits their dispatched work immediately when it builds clean. Don't batch.
- **Commit messages:** reference which proposal/phase. Format: `{Phase/proposal slug}: {short summary}`.
- **Status updates:** update this doc when ownership shifts, when a phase gate passes, or when a new coordination rule emerges.
- **Cross-orchestrator communication:** in lieu of direct messaging, use this doc or a small scratch section below ("Active cross-Opus notes") to flag things.

## Active cross-Opus notes

(append short notes here when one Opus needs another's attention)

- [Architect 2026-04-12 13:46] Onboarded. Watching w12/w13/w17 output for architectural review.
- [Architect → Opus 1 + PM 2026-04-12 13:46] **Phase 1 review — partial pass, two issues:**
  1. **Drift: crate location.** Plan (§Phase 1A) calls for a new `fireline-acp-ids` crate. What landed: `crates/fireline-semantics/src/ids.rs` module. `fireline-semantics` currently hosts pure TLA+-aligned state-machine kernels (liveness/stream_truth/session/approval/resume); mixing wire-level ACP identifiers into that crate muddies the boundary. Types themselves are clean re-exports of `sacp::schema::{SessionId, RequestId, ToolCallId}` plus `PromptRequestRef` / `ToolInvocationRef` — no synthetic identity, no branding, no correctness issue. Not a phase blocker. Recommendation: either (a) extract to the planned `fireline-acp-ids` crate before Phase 2 depends on it, or (b) PM updates the execution plan to reflect the chosen home. Prefer (a) — cleaner boundary, matches proposal intent.
  2. **Gap: `@fireline/state` not migrated.** Plan (§Phase 1B) explicitly required `packages/state/src/acp-types.ts` (new) + export from `@fireline/state`. Neither landed. `packages/state/src/index.ts` is untouched. Phase 6 (TS Schema Migration) needs these types consumed from `@fireline/state`, so this gap must close before Phase 6 dispatches — ideally as a follow-up micro-PR against the Phase 1 slice rather than carrying it forward. Flagging to Opus 1 for dispatch.

  Architectural verdict: Phase 1 client-side output is **clean and additive** (no synthetic ids leak in, `sacp::schema` types used correctly, pure re-exports). The work is fine as a foundation; the above are completion/alignment issues, not regressions. No veto — proceed once gap (2) is closed.

## Open architectural decisions (Opus 3 owns)

- Does `webhook-support.md` merge into `durable-subscriber.md` or stay standalone? Waiting on w15's proposal-index audit.
- Does `fireline.db()` session-flattened DB restructure in canonical-ids Phase 6 or post-refactor?
- Does "always-on" sandbox policy (from deployment proposal) ship with DurableSubscriber or as its own execution phase?
- Should chunk ordering strictly derive from durable-streams offset (as proposed) or can we keep a redundant `seq` field as a derived convenience? (Verification doc current answer: strict offset, no redundant field.)

## Known risks

1. Coordination drift if this doc isn't kept current — **Opus 2's responsibility to prevent**.
2. Agent-layer code changes landing during canonical-ids Phases 1.5–7 from non-refactor workstreams may cause merge conflicts. Opus 2 ensures their dispatches don't touch agent-layer source during those phases.
3. If TLC finds a spec bug in w17's work, phase-gate invariants may need adjustment — **Opus 3 reviews and propagates** to the execution plan.
4. Architectural decisions made in isolation (without Opus 3 review) may drift from ACP / durable-streams / conductor best practices. **Require Opus 3 sign-off** on any proposal before it becomes execution-ready.
