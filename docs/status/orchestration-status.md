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
| w10 | **Opus 2 (PM)** | onboarded — owns planning surface |
| w20 | **Opus 3 (Architect)** | onboarded — has posted Phase 1 review |
| w12 | Opus 2 → codex | **LANDED** — #4 at `bb01ce8`. Recycling for examples/ README refresh. |
| w13 | Opus 1 → codex | **returned to Opus 1 14:30** after landing #5 (`045fbba`) + B6 (`ad8ab78`). Now on Opus 1 orphan cleanup + Phase 1.5 prep. |
| w17 | Opus 2 → codex | **LANDED** — #7 at `1f3c610`. Recycling for proposal-index.md refresh with resolved-status updates. |
| w15 | Opus 2 → codex | **LANDED** — #6 at `57c144e` (webhook merge + SUPERSEDED banner). Recycling for doc/guide refresh. |
| w18 | Opus 2 → codex | **LANDED** — #2 at `3fa956e` (canonical ACP ids + plane-separated `fireline.db()`). Recycling for README staleness. |
| w19 | Opus 2 → codex | **STALLED** — cargo test still running. Redirecting to CI-first directive next. |
| w21 | Opus 1 → codex | **ACTIVE** — Phase 1 fixups (fireline-acp-ids crate + @fireline/state shim) |
| w22 | Opus 2 → codex | **LANDED** — #3 at `12bb7cd` (child_session_edge stripped; `_meta` trace context lineage). Recycling for approval-workflow README. |

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

## Master Roadmap

This is the top-level ordering view across the major proposal tracks. The table
shows execution order and rough size; the graph shows blocking structure. The
detailed phase plans still live in the sibling proposal and execution docs.

| Milestone | Phases | Status | Blocked By | Rough effort |
|---|---|---|---|---|
| ACP Canonical Identifiers | Phases 0-8 | Active. Phase 1 landed; Phase 1.5 dispatched; Phases 2-8 remain. | Internal phase gates only. | ~8 phases total; roughly 2-5 days per phase |
| DurableSubscriber implementation | Design only; execution plan pending | Not started. Proposal stable, implementation not yet sequenced. | ACP Canonical Identifiers through Phase 5 | TBD; expected multi-phase implementation, to be formalized by w17 |
| Durable Promises | Design only; execution plan pending | Not started. Proposal stable, imperative sugar only. | DurableSubscriber implementation | TBD; expected smaller follow-on after subscriber core, to be formalized by w18 |
| CLI production-readiness (`npx fireline deploy`) | Gap analysis + `fireline deploy` + always-on wiring | Execution plan pending. Base `npx fireline run` already in `packages/fireline/`. | Compose/start stabilization + DurableSubscriber always-on profile | Small per Opus 1 rescope (2026-04-12) — ~200-line gap doc |
| Hosted Fireline deployment | ~5-6 phases | **PAUSED 2026-04-12 14:55** — w15 stood down. Scope reframing: user flagged cloud-provider-AGNOSTIC, not Fly-specific. New scope: portable OCI image, deployment targets = any cloud container platform (Fly, Railway, Cloudflare Containers, microsandbox images, self-hosted Docker). Architect reframing direction now. | CLI production-readiness, DurableSubscriber catalog | ~5-6 phases once scope re-locked |
| Fleet UI positioning | Decision doc + minor wiring | Pending. `examples/flamecast-client/` is a working UI today. | canonical-ids Phase 4 (trace-context) for OTel visualization | Small per Opus 1 rescope (2026-04-12) — positioning + OTel wiring |
| ACP registry client | `crates/fireline-tools/src/agent_catalog.rs` (currently TODO stub) + `fireline-agents add <id>` CLI | Execution plan pending. Unlocks `agent(['pi-acp'])` against public ACP registry. | Independent track | Small — ~3 phases |
| Demo delivery synthesis (`pi-acp-to-openclaw`) | Narrative + E2E verification + anthropic-provider wiring validation | Orchestrator doc pending. | All above | ~1-2 focused polish phases; absorbs former item (6) anthropic-provider validation |

```text
ACP Canonical Identifiers
  ├──> DurableSubscriber
  │       └──> Durable Promises
  ├──> CLI maturity
  └──────────────────────────────┐
                                 ├──> Demo polish (pi-acp-to-openclaw)
DurableSubscriber ───────────────┤
Durable Promises ────────────────┤
CLI maturity ────────────────────┘
```

How to read this roadmap:

- The table is the sequencing view: what lands first, what is active now, and
  the rough size of each milestone.
- The graph is the blocking view: which milestones can proceed independently
  and which ones are downstream of canonical-ids stabilization.
- The execution detail does not live here. Use the sibling proposal docs for
  actual phase definitions, verification gates, and task decomposition.

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

## Contention rules

> Established 2026-04-12 by Opus 1 + Opus 3 after w19/w21 cargo target lock collision.
> **Tightened 2026-04-12 14:33 by Opus 1** after observing that `cargo check --workspace` itself contends on the shared `target/` lock.

**The shared `target/` directory on the main worktree is single-writer. Code-path codexes do not write to it at all. CI is the authoritative compile + test environment.**

Dispatch contract for every code-path codex:

1. Make the code change.
2. **Skip all local cargo commands.** Do not run `cargo check`, `cargo build`, `cargo test`, or anything else that touches `target/` on the shared worktree.
3. Commit + push (to a throwaway branch or directly to main per the phase's policy).
4. CI (GitHub Actions) runs `cargo check --workspace` + full test suite.
5. Gate on CI green via `gh run list --limit 1 --json status,conclusion,url`, then `gh run watch <run-id>`.

**The entire point of CI-first is that two codexes never run cargo concurrently against the same `target/`.** That is what was causing every "target lock contention" symptom we saw up through w19/w21.

**Narrow carve-out — isolated `CARGO_TARGET_DIR` (debugging only):**

If a codex absolutely must run cargo locally to debug (rare — only when CI iteration is too slow), it **must** use `CARGO_TARGET_DIR=/tmp/fireline-{codex-id}` to isolate itself from the shared `target/`. w19 and w21 did this correctly when they needed to debug. Default assumption: no local cargo. Owning Opus flags any exception in Active cross-Opus notes.

**Implication for the canonical-identifiers execution plan §Working Rules #2:** "green locally and in CI" now means **"green in CI"** — no local cargo step. Execution doc currently reads "`cargo check` green locally + CI tests green"; needs a follow-up patch to drop the local step.

**For PM dispatches:** doc-only work has no cargo footprint and is always safe-parallel. Only code-path dispatches need the CI-first instructions.

**For Opus 1 phase dispatches:** Opus 1 handles updating their own Phase N prompts to reflect "do not run cargo locally; push and check CI".

**Applies to review feedback too:** if a landed PR ran any local cargo command in its dispatch contract, note the deviation in the landed review. Future dispatches skip local cargo entirely.

## Active cross-Opus notes

(append short notes here when one Opus needs another's attention)

- `[PM 2026-04-12 13:47] Onboarded. Monitoring w15/w18/w19.`
- `[PM 2026-04-12 13:47] w15 LANDED as 9b89496 (proposal-index.md, 293 lines). Full drift catalog. 3 Critical + 2 Design + 1 Merge. See PM dispatch queue below.`
- `[PM 2026-04-12 13:47] w18 LANDED at e0a14c5 (pushed). Examples idiomatic + shared/wait.ts. Staging guide/README refresh pending Phase 1.5.`
- `[PM → Opus 3 2026-04-12 13:47] Acknowledging your Phase 1 review. Issue (1) crate location: prefer-(a) (extract to fireline-acp-ids) is an execution-plan decision; routing to Opus 1 to sequence. Issue (2) @fireline/state gap: routing to Opus 1 as a micro-PR against w12 before Phase 6 dispatches. If Opus 1 picks (b) instead, I'll patch execution doc. Flagging both in risks below.`
- `[PM → Opus 3 2026-04-12 13:47] proposal-index §5.6 recommends folding webhook-support.md into durable-subscriber.md. Not urgent — but my drift-fix #1 (durable-subscriber.md CrossSessionKey) benefits from knowing the answer. Default if you don't weigh in before I dispatch: keep standalone, dispatch merge as separate queue item.`
- `[PM → Opus 1 2026-04-12 13:47] See PM dispatch queue. My drift-fix work is doc-only (docs/proposals/*.md). Safe to run concurrent with Phases 1/1.5/2 — no agent-layer touches, no merge conflicts. Also flagging Opus 3's Phase 1 review (2 completion issues) for your sequencing.`
- `[PM 2026-04-12 13:50] Dispatched Critical drift fixes: #1 durable-subscriber → w15 (recycled), #2 platform-sdk → w18 (recycled), #3 client-api-redesign → w22 (claimed). All doc-only. Webhook-merge kept standalone per Opus 3 default; will queue as separate item once Opus 3 decides.`
- `[PM 2026-04-12 14:00] Absorbed Architect's MERGE decision on webhook-support.md → durable-subscriber.md. Dispatch queue row #6 updated: status now "gated on #1 landing" because both patches touch durable-subscriber.md. When w15 commits, I dispatch #6 as a sequential follow-on (same workspace recycle).`
- `[PM → Opus 1 2026-04-12 14:00] Ack on w21 progress (fireline-acp-ids crate + packages/state/src/acp-types.ts in worktree). No action from me. Will watch w19 for a stall signal separately and flag if it runs past 20m. Currently at ~15m on cargo test control_plane_push.`
- `[PM → Opus 3 2026-04-12 14:00] Logged your MERGE decision. Queue row #6 now reads: "dispatch after #1 lands". Same-file sequencing avoids merge conflicts on durable-subscriber.md.`
- `[PM 2026-04-12 14:05] w15 LANDED drift-fix #1 at 7551cb5 (durable-subscriber.md CrossSessionKey stripped, verification grep empty). Dispatched #6 (webhook merge, full API preservation + W3C trace context propagation) into w15 recycle.`
- `[PM 2026-04-12 14:05] Dispatch tooling learning: cmux send pastes long prompts but does NOT auto-submit. Must follow with cmux send-key --key Enter. w18/w22 sat idle for ~15 min before I noticed and sent Enter. New dispatch pattern: send prompt, then immediately send-key Enter. Building this into future dispatches.`
- `[PM → Opus 1 2026-04-12 14:05] Capacity: 7/10 codexes active (my w15/w18/w22 all on doc work; your w12/w21 on code; w19 stalled). IDLE: w13/w17 (yours — finished prior tasks). My three are saturated on #2/#3/#6. Drift #4/#5 stay queued until #2 or #3 lands. Execution doc §Working Rules #2 patched for CI-first directive.`
- `[PM → Opus 1 2026-04-12 14:05] FLAG: w12 cargo test running 30+ min (past your 20m threshold). And w19 stalled at 15min+ on cargo test control_plane_push. Per new CI-first directive, both should be redirected: stop local cargo test, commit check-green result, let CI run suite. Your call whether to intervene now or let them drain.`
- `[PM 2026-04-12 14:15] Claimed w12/w13/w17 per Opus 1 handover. Rethought #4/#5 defer: they edit different files from #2/#3, no same-file conflict, so dispatching now. Added queue row #7 for a low-priority exploration cleanup (managed-agents-mapping.md:231) to fill the third slot. w17 → row #7, w12 → #4, w13 → #5. Capacity now targets 9/10.`
- `[PM → Opus 1 2026-04-12 14:15] Ack on Phase 1 variant situation. Understand w12's 3a75a06 is superseded by w21's forthcoming fixup; not touching that commit. Reclaimed w12 for doc work (#4). Will not dispatch Phase 1.5 — that stays yours when Architect reconfirms.`
- [Architect 2026-04-12 13:46] Onboarded. Watching w12/w13/w17 output for architectural review.
- [Architect → Opus 1 + PM 2026-04-12 13:46] **Phase 1 review — partial pass, two issues:**
  1. **Drift: crate location.** Plan (§Phase 1A) calls for a new `fireline-acp-ids` crate. What landed: `crates/fireline-semantics/src/ids.rs` module. `fireline-semantics` currently hosts pure TLA+-aligned state-machine kernels (liveness/stream_truth/session/approval/resume); mixing wire-level ACP identifiers into that crate muddies the boundary. Types themselves are clean re-exports of `sacp::schema::{SessionId, RequestId, ToolCallId}` plus `PromptRequestRef` / `ToolInvocationRef` — no synthetic identity, no branding, no correctness issue. Not a phase blocker. Recommendation: either (a) extract to the planned `fireline-acp-ids` crate before Phase 2 depends on it, or (b) PM updates the execution plan to reflect the chosen home. Prefer (a) — cleaner boundary, matches proposal intent.
  2. **Gap: `@fireline/state` not migrated.** Plan (§Phase 1B) explicitly required `packages/state/src/acp-types.ts` (new) + export from `@fireline/state`. Neither landed. `packages/state/src/index.ts` is untouched. Phase 6 (TS Schema Migration) needs these types consumed from `@fireline/state`, so this gap must close before Phase 6 dispatches — ideally as a follow-up micro-PR against the Phase 1 slice rather than carrying it forward. Flagging to Opus 1 for dispatch.

  Architectural verdict: Phase 1 client-side output is **clean and additive** (no synthetic ids leak in, `sacp::schema` types used correctly, pure re-exports). The work is fine as a foundation; the above are completion/alignment issues, not regressions. No veto — proceed once gap (2) is closed.

## PM dispatch queue (post-w15 landing)

**ALL 7 DRIFT ITEMS LANDED as of 14:24.** Queue below retained for audit trail.

Drift fixes from `docs/proposals/proposal-index.md §6`, priority order:

| # | Priority | Proposal | Drift | Status |
|---|---|---|---|---|
| 1 | Critical | `durable-subscriber.md` | `CrossSessionKey` / `cross_session` completion shape; replace with caller-local `PromptKey(SessionId, RequestId)` / `ToolKey(SessionId, ToolCallId)`; move cross-session causality to `_meta` trace context. Line ranges `66-70`, `154-157`, `321-327`, `393-401`, `447`. | **LANDED at `7551cb5`** (14:03) |
| 2 | Critical | `platform-sdk-api-design.md` | `string` ACP ids + infra rows (`PromptTurnRow`, `ConnectionRow`, `TerminalRow`, `RuntimeInstanceRow`) in `fireline.db()`. Swap to `sacp::schema` branded types, rename prompt-turn → prompt-request, move infra rows to admin API. Lines `114-115`, `151-198`, `215`, `395-402`. | **LANDED at `3fa956e`** (14:17) |
| 3 | Critical | `client-api-redesign.md` | `child_session_edge` rows + single-tenant stream as lineage. Switch to prompt-request + `_meta` trace context. Lines `190`, `363`, `422`, `437`, `442-475`. | **LANDED at `12bb7cd`** (14:18) |
| 4 | Design | `unified-materialization.md` | `ActiveTurnIndex` / `prompt_turn` as steady state. Rewrite around `SessionIndex`/`HostIndex`. Lines `14`, `89-100`. | **LANDED at `bb01ce8`** (14:24) |
| 5 | Design | `secrets-injection-component.md` | Rust `session_id: String` in session-scoped keys + audit events. Type as `sacp::schema::SessionId`. Lines `147`, `531`. | **LANDED at `045fbba`** (14:23) |
| 7 | Low | `docs/explorations/managed-agents-mapping.md:231` | Marked `ActiveTurnIndex` transitional with pointer to canonical-identifiers.md. | **LANDED at `1f3c610`** (14:22) |
| 6 | Design | `webhook-support.md` → `durable-subscriber.md` | **MERGE** — absorb `webhook-support.md §6` into `durable-subscriber.md §5.2`; mark SUPERSEDED in `proposal-index.md`. | **LANDED at `57c144e`** (14:12) |

Post-w19:
- Commit + push; verify CI green; log commit sha here.

## PM backlog dispatch (2026-04-12 14:22)

Post-landings recycle (w15/w18/w22 now idle):

| # | Target | Task | Dispatched to |
|---|---|---|---|
| B1 | `docs/guide/` | Add references to new proposals (`acp-canonical-identifiers.md`, `durable-subscriber.md`, `durable-promises.md`) in the guide README and linkbacks from relevant guide pages. Proposal-level language, no code vocabulary changes. | w15 |
| B2 | `README.md` | Staleness check against current main — flag / fix any examples or collection names that drift from today's API surface. | w18 |
| B3 | `examples/approval-workflow/README.md` | Reframe approval narrative to reference the DurableSubscriber substrate + `awakeable`/`resolveAwakeable` mental model from `durable-promises.md`. Keep current code examples accurate. | w22 |
| B4 | `w19` | Redirect per CI-first contention rule: stop local cargo test, commit check-green result, push, let CI run suite. | will send to w19 |

Deferred (background / blocked):
- `docs/guide/` + `README.md` canonical-ids vocabulary refresh — blocked on Phase 1.5 landing (vocabulary not stable yet).
- `docs/demos/pi-acp-to-openclaw.md` canonical-ids + durable-promises rewrite — blocked on Phases 2 + 5 + DurableSubscriber primitive.

## Open architectural decisions (Opus 3 owns)

- ~~Does `webhook-support.md` merge into `durable-subscriber.md` or stay standalone?~~ **DECIDED 2026-04-12 (Architect): MERGE.** `durable-subscriber.md §5.2` already specifies `WebhookSubscriber` as a primary use-case of the generalized primitive; `webhook-support.md`'s mechanism (host-side always-on stream subscriber, at-least-once delivery, cursor-stream persistence, topology component lowering) is a strict subset of DurableSubscriber's contract. DurableSubscriber additionally mandates W3C Trace Context propagation on outbound side effects — webhook-support.md lacks this, so merging is a strict architectural improvement, not just dedup. Merge plan: absorb webhook-support.md's concrete API surface (§6 `webhook()` middleware helper + topology lowering + host target config) into durable-subscriber.md as a dedicated subsection under §5.2, and mark `webhook-support.md` as SUPERSEDED in `proposal-index.md`.
- Does `fireline.db()` session-flattened DB restructure in canonical-ids Phase 6 or post-refactor?
- ~~Does "always-on" sandbox policy (from deployment proposal) ship with DurableSubscriber or as its own execution phase?~~ **DECIDED 2026-04-12 (Architect): ships AS a DurableSubscriber profile.** `lifecycle.alwaysOn = true` is exactly the DurableSubscriber shape: event = `deployment_wake_requested` (on-boot scan or heartbeat), key = `SessionId` (deployment identity), completion = `sandbox_provisioned` (with runtime id), mode = active. The TLA wake invariants already in `managed_agents.tla` (`WakeOnReadyIsNoop`, `WakeOnStoppedChangesRuntimeId`, `WakeOnStoppedPreservesSessionBinding`, `ConcurrentWakeSingleWinner`, `SessionDurableAcrossRuntimeDeath`) are the exact semantic base an always-on DurableSubscriber needs. No new primitive. Phase: lands with DurableSubscriber rollout; add a dedicated `AlwaysOnDeploymentSubscriber` implementation section to `durable-subscriber.md §5`.
- **NEW — hosted Fireline deployment (preliminary direction, Architect 2026-04-12; AMENDED after Opus 1 reframe)** for PM queue item (7) `hosted-fireline-deployment.md`:
  - **(a) Fireline host packaging: portable OCI image.** The host process ships as a standard container image. Supported deployment targets are any platform offering (1) long-running containers, (2) attached persistent storage for durable-streams data, (3) inbound HTTP/SSE for the ACP proxy. Named candidates: Cloudflare Containers, Fly.io, Railway, Render, self-hosted Docker Compose, Kubernetes, bare VM with Docker. This avoids provider lock-in, matches the demo narrative ("deploy your agent fleet to whatever cloud you already use"), and lets `crates/fireline-sandbox/src/microsandbox.rs` packaging patterns carry over. **Rejected: Vercel serverless** — long-lived SSE ACP proxy fights the request/response model. **Rejected for MVP: raw K8s as the *only* target** — should be supported but not required. Validation requirement: each listed target must be smoke-tested for long-lived ACP SSE before it enters the "supported" list.
  - **(b) Durable-streams deployment model: sidecar by default.** For production, durable-streams runs as an independent service (separate container, separate lifecycle) alongside the Fireline host on the same substrate, backed by attached persistent storage, multi-tenant by namespace. Independent lifecycle is load-bearing: host restart must not restart durable-streams, because `SessionDurableAcrossRuntimeDeath` and the plane-separation invariants require state to outlive the host. **Bundled single-image variant** allowed for quickstart/local-dev/single-node demos only — must carry a deploy-warning that it erases the durability guarantee on host crash with local disk. Multi-host durability requires the sidecar model.
  - **(c) Cloud sandbox provider — ORTHOGONAL to (a)/(b).** This decision is about where the AGENT runs, not where Fireline runs. Direction unchanged from previous version: **Anthropic managed agents as PRIMARY** for demo target; microsandbox + Docker kept as secondary for full code-exec; provider abstraction preserves multi-provider capability. Host packaging and sandbox provider are independent axes — users can run Fireline in OCI on Cloudflare Containers and dispatch to Anthropic managed agents, or run Fireline on Fly.io and dispatch to microsandbox, etc.
  - **Dispatch guidance:** PM can queue a codex against this direction to draft `hosted-fireline-deployment.md` as an execution plan. Each sub-decision is revisitable inside the draft without re-opening the architectural direction. I'll review the draft when it lands.
- Should chunk ordering strictly derive from durable-streams offset (as proposed) or can we keep a redundant `seq` field as a derived convenience? (Verification doc current answer: strict offset, no redundant field.)

## Known risks

1. Coordination drift if this doc isn't kept current — **Opus 2's responsibility to prevent**.
2. Agent-layer code changes landing during canonical-ids Phases 1.5–7 from non-refactor workstreams may cause merge conflicts. Opus 2 ensures their dispatches don't touch agent-layer source during those phases.
3. If TLC finds a spec bug in w17's work, phase-gate invariants may need adjustment — **Opus 3 reviews and propagates** to the execution plan.
4. Architectural decisions made in isolation (without Opus 3 review) may drift from ACP / durable-streams / conductor best practices. **Require Opus 3 sign-off** on any proposal before it becomes execution-ready.
