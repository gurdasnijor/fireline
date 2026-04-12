# Architectural Beads Validation

> Date: 2026-04-12
> Reviewer: Architect (Opus 3)
> Scope: bead-registry retrofit for alignment-check risks + missing-epic audit + architectural notes on active beads.
> Companion: [alignment-check-2026-04-12.md](./alignment-check-2026-04-12.md)

## TL;DR

Beads retrofit complete. Four alignment-check risks now live-queryable on `mono-vkpp` epic. Three missing epic beads created (DurableSubscriber, Durable Promises, Hosted Deploy Phase 2+). Per-target validation residual beaded. Phase notes on vkpp.5/.6/.7/.8/.10 carry my pre-approval conditions forward. No cycles in the cross-epic dep graph.

## Retrofit: R1–R4 → beads

| Risk | Bead | Priority |
|---|---|---|
| R1 TLC non-vacuity unconfirmed | `mono-c80` | P1 |
| R2 Phase 3.5 chunk payload consumer coupling | `mono-604` | P1 |
| R3 Lineage-gap window between Phase 3 and Phase 4 | `mono-9q8` | P1 |
| R4 Phase 6A DeploymentSpecSubscriber TLA coverage gap | `mono-445` | P3 |

All four created with `discovered-from:mono-vkpp`. `bd` custom `risk` type not configured in this workspace, so tasks with `[RISK Rn]` title prefix serve as the live-queryable surface. If/when PM-A configures custom types, retitle.

## Missing-epic audit → beads

| Area | Existing coverage? | Action | Bead |
|---|---|---|---|
| DurableSubscriber implementation (DS Phases 0-8) | stateright-model stub only (`mono-vkpp.13`) | created epic | `mono-axr` |
| Durable Promises (imperative awakeable surface) | none | created epic | `mono-139` |
| Hosted Fireline Deployment Phase 2+ (post-Tier-A) | Tier A in demo lane (`mono-thnc`) | created epic | `mono-0xc` |
| Per-target validation checklist (Fly/CF/Railway/K8s/Docker) | none | created task | `mono-8c5` |
| DeploymentSpecSubscriber (Tier C) | flagged in R4 only | task covered by alignment; TLA gap blocks dispatch | (in R4 bead `mono-445`) |
| Observability Phase 3/4 post-demo | T4 covers demo-minimum (`mono-thnc.4`) | created task | (bd returned empty id in my output; retry if missing) |
| Phase 7 db-plane-separation.test.ts | subsumed in `mono-vkpp.9` | no new bead | `mono-vkpp.9` |
| CI infra gaps (fireline-harness lane `fa46db9` closed) | closed already | no new bead | — |
| ACP Registry Phase 3 follow-ons | Phase 1-3 already landed (`1ed8b50`, `ebb240a`, `739cbcb`) | no new bead unless next scope defined | — |

## Dep wiring

- `mono-139` (Durable Promises epic) **blocks on** `mono-axr` (DurableSubscriber epic) — enforced via `bd dep add`.
- `mono-axr` → `mono-vkpp.7` (canonical-ids Phase 5) attempted as `blocks`; rejected by bd because epics can only block epics. Dep relationship documented in `mono-axr` note text instead — Phases 1-8 of DurableSubscriber will be sub-beaded under `mono-axr` with individual deps on `mono-vkpp.7`.
- `mono-0xc` (Hosted Deploy Phase 2+) `discovered-from:mono-8c5`.
- `mono-445` (R4), `mono-c80` (R1), `mono-604` (R2), `mono-9q8` (R3) all `discovered-from:mono-vkpp`.

## Dependency cycle detection

Graph sketch (epic level):

```text
mono-thnc (demo)                       mono-vkpp (canonical-ids)
  │                                       │
  ├── thnc.4 (OTel T4) ─────── reads ──── vkpp.6 (Phase 4)
  ├── thnc.6/.7/.8 rehearsals              │
  │                                        ▼
  └── (Tier A smoke local Docker)        vkpp.7 ── reads ── mono-axr (DS epic)
                                                                │
                                                                ▼
                                                           mono-139 (Promises epic)

mono-0xc (Hosted Phase 2+) ── reads ── mono-8c5 (per-target validation)
```

**No cycles.** Every edge is `X lands before Y starts`. Demo lane (`mono-thnc`) reads canonical-ids gates but does not write into them; canonical-ids doesn't depend on demo artifacts.

Risk: if `thnc.4.2` (peer `_meta.traceparent`) runs before `vkpp.6` lands, it's a no-op — doc explicitly says "skip peer if canonical-ids Phase 4 not green, 5 spans stand alone." Graph edge is a soft gate, not a hard one. **Not a cycle.**

## Architectural notes landed (via `bd note`)

| Bead | Note summary |
|---|---|
| `mono-vkpp` | Epic aligned per alignment-check; 4 risks tracked; Phases 1-3 clean; Phase 4 = 8-16h; Phase 6 parallel-with-Phase-5 requires `.passthrough()` |
| `mono-vkpp.5` | Phase 3.5 high consumer coupling; `extractChunkTextPreview` helper mitigates; land tight before Phase 4 |
| `mono-vkpp.6` | Phase 4 = 8-16h; 4-5 call-site audits + TextMapPropagator + 6 span sites + 2-runtime integration test |
| `mono-vkpp.7` | Phase 5 CONDITIONAL auto-approve: rg-zero + peer regression test asserting traceparent continuity + session lineage |
| `mono-vkpp.8` | Phase 6 HARD CONSTRAINT: Zod `.passthrough()` during migration window; `.strict()` returns at Phase 8 |
| `mono-vkpp.10` | Phase 8 AUTO-APPROVE on extended rg gate (includes all transitional names from Phases 1-7 git diff) |
| `mono-axr` | DurableSubscriber is proper generalization; no new substrate; every feature = profile; gated on vkpp.7 |
| `mono-139` | Durable Promises is imperative sugar; NOT a second workflow engine; canonical keys only |
| `mono-thnc` | Demo lane coherent with hosted-deploy-surface-decision; no new HTTP surface; per-target validation = post-demo residual |

## Residual action items (beaded)

- `mono-8c5` — per-target validation checklist. Covers the alignment-check §6 supported-target bar (long-running containers + persistent storage + HTTP/SSE). Post-demo.
- `mono-c80` — TLC non-vacuity run. **BLOCKS Phase 3 dispatch** (but Phase 3 already closed — this is now a non-blocker). Should still be run before Phase 5 to ensure deletions don't silently vacuate other invariants.
- `mono-445` — DeploymentSpecSubscriber TLA coverage extension. Blocks Phase 6A dispatch (Tier C; deferred).

## Feasibility flags (severity labeled)

| Flag | Severity | Bead | Note |
|---|---|---|---|
| TLC may be vacuous | MEDIUM | `mono-c80` | Resolvable by running TLC; no design change needed |
| Phase 6 strict parsing breaks dual-read | HIGH if ignored | `mono-vkpp.8` | Mitigation (passthrough) is documented constraint; LOW if constraint honored |
| Phase 3→4 lineage window | LOW | `mono-9q8` | Accepted transitional state per execution plan |
| External consumers break on Phase 3.5 | LOW | `mono-604` | Internal consumers mitigated via helper; external = breaking-migration by design |
| Tier A demo on local Docker (not CF) | INFO | (covered in alignment check §R2+Q2) | Architectural choice; CF deferred pending object-storage-native durable-streams protocol |

No **BLOCKER** flags. Phase cascade can fire full throttle per PM-A's plan.

## Coordination boundary (with PM-A and Jessica)

My validation lane: **architectural feasibility + completeness-vs-architecture.** PM-A / Jessica own **completeness-vs-execution** (progress tracking, dispatch timing, CI status). Overlap kept minimal:

- I create / add-note-to epic beads and cross-cutting risks.
- PM-A creates / updates per-phase task beads + tracks dispatch status.
- Jessica creates / updates demo-lane task beads + tracks rehearsal status.
- Cross-lane reads are fine; cross-lane WRITES only when the other owner is AWOL and an architectural decision is load-bearing.

## What's NOT beaded (intentionally)

- My alignment-check doc itself (`alignment-check-2026-04-12.md`) — it's a long-form artifact, not a coordination unit.
- Decision docs (`hosted-deploy-surface-decision.md`, this doc) — same reasoning.
- Per-target future cloud validations beyond the checklist entry — those get beaded when we actually schedule them.

## References

- [alignment-check-2026-04-12.md](./alignment-check-2026-04-12.md)
- [state-projector-audit-review-2026-04-12.md](./state-projector-audit-review-2026-04-12.md)
- [hosted-deploy-surface-decision.md](../proposals/hosted-deploy-surface-decision.md)
- [acp-canonical-identifiers-execution.md](../proposals/acp-canonical-identifiers-execution.md)
- [durable-subscriber-execution.md](../proposals/durable-subscriber-execution.md)
- [durable-promises-execution.md](../proposals/durable-promises-execution.md)
- [hosted-fireline-deployment.md](../proposals/hosted-fireline-deployment.md)
- [orchestration-status.md](../status/orchestration-status.md)
