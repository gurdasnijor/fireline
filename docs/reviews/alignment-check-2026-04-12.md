# Architectural Alignment Check

> Date: 2026-04-12 (14:XX on a very long day)
> Reviewer: Architect (Opus 3)
> Scope: end-to-end coherence check across canonical-identifiers, DurableSubscriber + Promises, hosted deployment, CLI, observability, ACP registry, and the state-projector audit synthesis.

## TL;DR

**We are lined up.** Every major proposal and every landed code change traces back to the governing acceptance criterion in [`acp-canonical-identifiers.md`](../proposals/acp-canonical-identifiers.md), the plane-separation rule, and the "no new HTTP surface" decision. The phase DAG is acyclic, blockers are honored, primitives compose. Four real risks remain, all known and tracked — none are architectural regressions.

This review is load-bearing: if you accept its verdict, you can dispatch Phase 3 (post PM exec-plan patch `bfa222c`, already in) and Phase 6A `DeploymentSpecSubscriber` work without further architectural gate. If you reject any numbered finding below, re-open the governing doc before the next dispatch.

## 1. Governing Bar Check

**Acceptance criterion** (paraphrased from `acp-canonical-identifiers.md §Acceptance Criterion`):

> No synthetic or out-of-band identifiers on the agent plane. Only ACP schema identifiers (`SessionId`, `RequestId`, `ToolCallId`) + ACP `_meta` + derived storage keys that are pure concatenations.

**Status across landed + in-flight work:**

| Area | On the agent plane? | Canonical? | Notes |
|---|---|---|---|
| Session identity (`SessionRecord.session_id`) | ✅ | `SessionId` | Post-Phase-1.5 |
| Approval `RequestId` | ✅ | `RequestId` from actual JSON-RPC id (post-Phase-2) | `approval_request_id()` SHA256 path deleted |
| Prompt request rows (Phase 3) | will be | `(SessionId, RequestId)` | dual-write `prompt_request` entity |
| Chunk ordering (Phase 3 + 3.5) | will be | durable-streams offset | no `chunk_id` / `chunk_seq` / `seq` fields |
| Peer call result (`PeerCallResult.child_session_id`) | ✅ | `SessionId` (Phase 1.5 + 4) | |
| Trace context (Phase 4) | ✅ planned | W3C via `_meta.traceparent` + `tracestate` + `baggage` | replaces `_meta.fireline.*` |
| ActiveTurnIndex (Phase 5) | delete | replaced by canonical `SessionId` + trace context | |
| `child_session_edge` rows | delete (pulled into Phase 3) | — | zero agent-plane consumers; pure tech debt |
| Host identity (`host_key`, `runtime_id`, `node_id`, `provider_instance_id`) | ❌ — infra plane only | Fireline-minted, correct | never on agent-plane rows per plane-separation |
| Subscriber completion keys | ✅ | `CompletionKey::{PromptKey, ToolKey}` | composed only of canonical ACP types |
| Awakeable keys (promises) | ✅ | `PromptKey` / `ToolKey` / `PromptStepKey(SessionId, RequestId, StreamOffset)` | offset is not a Fireline counter |
| Deployment identity | ✅ | `SessionId` (per `AlwaysOnDeploymentSubscriber` / `DeploymentSpecSubscriber`) | no bespoke `deployment_id` |
| Webhook delivery keys | ✅ | `PromptKey` or `ToolKey` | merged into DurableSubscriber §5.2 |
| Observability attributes | ✅ | `fireline.session_id`, `fireline.request_id`, `fireline.tool_call_id` | no synthetic span ids in Fireline attrs |
| Tool catalog (`AgentCatalog`) | N/A | catalog names AGENT identities (`pi-acp`), not deployment instances | orthogonal to canonical-ids |

**Verdict:** every user-facing identity surface resolves to canonical ACP types or explicitly lives in the infra plane. No exceptions found.

## 2. Phase DAG Sanity

Dependency graph from `acp-canonical-identifiers-execution.md` + sibling execution plans:

```text
Canonical-Identifiers                               External tracks
─────────────────────                               ─────────────────
Phase 0 (docs) ─> Phase 1 (types)                   hosted-fireline-deployment
                    │                                   └── Phase 1 ───► Tier A OCI (landed 2c68794)
                    └─> Phase 1.5 (field renames)                       Phase 2+ = multi-region/provider
                           │                        fireline-cli-execution
                           └─> Phase 2 (approval)       └── Phase 1 reshape ───► build + push (cd13ccd)
                                  │                 observability-integration
                                  └─> Phase 3           └── Phase 1 ───► OTel bootstrap (ff9b712)
                                         │              Phase 2+ = spans + attributes
                                         └─> Phase 3.5   acp-registry-execution
                                                │           Phase 1 AgentCatalog (1ed8b50)
                                                └─> Phase 4 Phase 2 CLI (ebb240a)
                                                       │    Phase 3 compose integration (739cbcb)
                                                       └─> Phase 5
                                                              │
                                                              └─> Phase 6 ─> Phase 7 ─> Phase 8

DurableSubscriber
─────────────────
DS Phase 0 (docs) ─> DS Phase 1 (subscriber substrate)
                       │
                       ├─ gated on canonical-ids Phase 5
                       └─> DS Phase 2..6 (profiles)
                              │
                              └─> DS Phase 7 (TS helper) ─> Durable Promises Phase 1..5

DS Phase 6A (DeploymentSpecSubscriber)
  └─ deferred, Tier C only, not required for MVP

Hosted Deploy + CLI (Tier A MVP)
  └─ independent of canonical-ids sequence; blocks only on hosted-deploy-surface-decision (77e007d, landed)
```

**Checks:**

- ✅ No cycles. Each edge is a strict "X lands before Y starts" relationship.
- ✅ The "delete lineage structures AFTER adding W3C trace context" constraint is preserved: Phase 5 (delete `ActiveTurnIndex`) depends on Phase 4 (add W3C trace). `child_session_edge` deletion was correctly pulled up to Phase 3 because it had zero agent-plane consumers — the "don't delete before replacement" rule does not apply.
- ✅ DurableSubscriber implementation gated on canonical-ids Phase 5 — correct, because DS uses canonical `CompletionKey` which needs the canonical-ids refactor complete.
- ✅ Durable Promises gated on DS Phase 7 (TS helper) — correct, because Promises is the imperative projection and needs the substrate landed first.
- ✅ Hosted / CLI / OTel / Registry are parallelizable with canonical-ids. Coordination burden is low because they don't modify agent-plane row shapes.

**Finding:** DAG is clean. No phase-ordering bugs.

## 3. Primitive Composition

Fireline's current primitive inventory:

1. **Canonical ACP identifiers** (`SessionId`, `RequestId`, `ToolCallId`) via `fireline-acp-ids` crate + `@fireline/state` shim.
2. **Durable streams** as the truth plane (agent + infra separately).
3. **ACP proxy / conductor** as the protocol boundary.
4. **DurableSubscriber** as the generalized suspend/resume + active side-effect primitive.
5. **Durable Promises** as the imperative projection of DurableSubscriber.
6. **W3C trace context via `_meta`** as the lineage primitive (Phase 4+).
7. **Providers + sandboxes** as runtime substrate (unchanged layering).

**Composition check:**

| Composition | Works because |
|---|---|
| Approval gate = `ApprovalGateSubscriber` (DurableSubscriber profile) | Same `PromptKey(SessionId, RequestId)` + `DSV-10..13` substrate obligations |
| Webhook = `WebhookSubscriber` (active profile) | Same substrate + W3C propagation rule (`DSV-05`) |
| Always-on deployment = `AlwaysOnDeploymentSubscriber` (active profile) | Same substrate; TLA wake invariants are the semantic base |
| Tier C hosted spec = `DeploymentSpecSubscriber` (passive, Tier C) | Same substrate; key = `SessionId`; replay covered by `DSV-01..02` |
| Awakeable = passive subscriber + imperative handle | Promises is sugar over passive subscriber, NOT a second substrate |
| Peer call (post Phase 4) = ACP `session/prompt` + W3C `_meta` | Lineage is the trace tree, not a Fireline row |
| Local → hosted deploy (Tier A) = OCI image + target-native deploy + host boots with embedded spec | No Fireline-owned deploy API; provider + sandbox abstractions unchanged |
| Observability = OTLP export of Fireline spans with canonical ACP attrs | Same trace tree as ACP `_meta`; one lineage surface |

**Finding:** every primitive is either new substrate (DurableSubscriber) OR a profile on existing substrate. Nothing re-implements what another primitive already handles. The one architectural risk — that Durable Promises becomes a second workflow engine — is explicitly forbidden by the proposal (§3 line: "reuse the existing completion-envelope contract and CompletionKey type from the subscriber substrate") and mechanically enforced by the verification audits.

## 4. Invariant ↔ Code Change Coverage

Cross-walk of TLA+ canonical-ids invariants (from `managed_agents.tla`) + DSV-* (from `durable-subscriber-verification.md`) against the phases that actually touch the behavior they constrain:

| Invariant | Constrains | Phase(s) that exercise it | Gate |
|---|---|---|---|
| `AgentLayerIdentifiersAreCanonical` | all agent-plane rows | Phases 1.5, 2, 3, 3.5, 6 | Phase 3 `rg` gate + Phase 6 TS schema tests |
| `InfrastructureAndAgentPlanesDisjoint` | row-field distribution | Phases 3, 7 | Phase 7 new `db-plane-separation.test.ts` |
| `CrossSessionLineageIsOutOfBand` | lineage path | Phases 3, 4, 5 | Phase 4 `rg` for `_meta.fireline` + Phase 5 deletion gate |
| `ChunkOrderingFromStreamOffset` | chunk rows | Phase 3 (rekey) + Phase 3.5 (payload) | Phase 3 `rg` for `chunk_id|chunk_seq|seq` |
| `ApprovalKeyedByCanonicalRequestId` | approval gate | Phase 2 | Phase 2 completed (commit `074b14e` + `8d9d204`) |
| `DSV-01 CompletionKeyUnique` | subscriber + awakeable | DS Phase 1, Promises Phase 1 | TLA+ + Stateright |
| `DSV-02 ReplayIdempotent` | replay | DS Phase 1, Promises Phase 4 | Stateright + migration fixtures |
| `DSV-03 RetryBounded` | active subscribers | DS Phase 3 (webhook), Phase 5 (peer) | TLA+ + Stateright |
| `DSV-04 DeadLetterTerminal` | active subscribers | DS Phase 3..5 | TLA+ + audits |
| `DSV-05 TraceContextPropagated` | all outbound side effects | DS Phase 3..5, canonical-ids Phase 4 | mechanical audit (`trace-propagation-audit`) |
| `DSV-10..13` (approval substrate) | substrate invariants | DS Phase 2 migration fixtures | Preserved from approval-gate-correctness |

**Finding:** every invariant has a phase that exercises it AND a gate that mechanically verifies it. No orphan invariants. No invariant-free phases.

**Pending:** TLC manual run against `ManagedAgentsCanonicalIds.cfg` (w17 followup) — must land before Phase 3 dispatches to confirm invariants are non-vacuous. This is a known gate; not a new risk.

## 5. Plane Separation Audit

Quick inventory of where plane boundaries are held today:

| Layer | Agent plane? | Infra plane? | Boundary integrity |
|---|---|---|---|
| `fireline.db()` | ✅ (Phase 7 enforces) | ❌ (host rows move to admin API) | post-Phase-3 move + post-Phase-7 test |
| Rust `SessionRecord` | agent-plane fields ✅ + transitional infra fields | — | Phase 7 deletes transitional infra fields |
| Durable streams | `state/session/{session_id}` agent-plane | `hosts:tenant-{id}`, `sandboxes:tenant-{id}`, `specs:tenant-{id}` | stream-namespace boundary |
| `_meta` | reserved W3C keys + explicit ACP ext | — | single surface for cross-plane carriage |
| Subscriber config + retry state | — | `subscribers:tenant-{id}` | per `durable-subscriber.md §3.5` |
| AgentCatalog | catalog is identity registry — agent plane | — | NOT a deploy registry (decision doc §4) |

**Finding:** plane separation is enforced at design time in every doc, and will be enforced at compile time / test time by Phase 7 + the `db-plane-separation.test.ts` gate. `child_session_edge` was the one boundary violator (mixed `parent_host_id` infra + `parent_session_id` agent in one row); its deletion in Phase 3 closes that specific seam.

**Pending:** `HostInstanceRow` move to `hosts:tenant-{id}` in Phase 3 must land for the boundary to be clean pre-Phase-7.

## 6. Surface Area Discipline

User's standing directive: no new HTTP surface unless strictly necessary.

**Current surface inventory:**

- ACP proxy (existing)
- durable-streams HTTP/SSE (existing, external library)
- host `/healthz` (existing, operational)
- control plane `fireline.admin.*` (existing, operational — sandbox provision/destroy)

**Surfaces we considered and rejected:**

- `PUT /v1/deployments/{name}` — rejected in `hosted-deploy-surface-decision.md §e`.
- `PUT /v1/specs/{name}` — rejected in the same decision.
- Fleet UI product surface — retracted (observability replaces it).
- Deploy-time CLI flags as config (`--always-on`, `--peer`, `--durable-streams-url`, `--model`) — rejected in `deployment-and-remote-handoff.md §4`.

**Finding:** zero new HTTP surface introduced. All deployment actions go through existing substrates (OCI packaging + target-native tooling for Tier A; durable-streams append for Tier C). The CLI rescopes (`fireline build`, `fireline push`) are thin codegen / durable-streams-append wrappers, not new protocols.

## 7. Open Risks / Gaps

Four real risks. All tracked; none are architectural regressions.

### R1. TLC non-vacuity unconfirmed

`ManagedAgentsCanonicalIds.cfg` invariants may be trivially true if the spec's current `Next` relation doesn't populate `AgentRows`. Until w17 runs TLC, we don't know. **Phase 3 dispatch is gated on this.** Impact if vacuous: we'd need to expand the spec actions before Phase 3 can use the invariants as a real gate.

### R2. Chunk Payload Redesign (Phase 3.5) consumer coupling

Flamecast, `examples/multi-agent-team`, `examples/live-monitoring`, and `turn-chunks.ts` all pattern-match `chunk.type` and `chunk.content`. Phase 3.5 rewrites these to `SessionUpdate` variant matching. **Mitigation:** the one-phase TS migration helper (`extractChunkTextPreview`) keeps simple examples working; the rewrite is coordinated in a single commit. **Residual risk:** app developers outside this repo who consumed old shape break. Acceptable given the canonical-ids-is-a-breaking-migration framing.

### R3. Transitional state between Phase 3 and Phase 4

Phase 3 deletes `InheritedLineage` parsing; Phase 4 adds W3C trace context. Between them, peer calls lose trace-lineage propagation entirely. **Explicit in the execution plan** (§Adjusted Phase Order) and accepted as transitional. Impact: observability spans lose parent-child relationships for peer calls during the window. **Mitigation:** land Phases 3 + 3.5 + 4 as a tight sequence; don't leave Phase 3 sitting on main for days.

### R4. Phase 6A DeploymentSpecSubscriber verification is not TLC-proven yet

`DeploymentSpecSubscriber` is specified in `durable-subscriber-verification.md` to inherit `DSV-01` / `DSV-02` coverage, but no TLA+ model for spec-stream materialization exists yet. **Acceptable** because Phase 6A is Tier C (deferred); Tier A MVP doesn't need it. Before Phase 6A dispatches, extend the DurableSubscriber TLA model to include `deployment_spec_published` / `spec_loaded` actions.

## 8. Things I Checked That Aren't Risks

For completeness — these are areas I verified and found clean:

- **`_meta.traceparent` timing**: proposal has Phase 4 adding it before Phase 5 deletes lineage. OK.
- **`chunk_v2` payload shape during Phase 3 dual-write**: explicitly kept on OLD string-typed shape until Phase 3.5 independently rewrites it. Preserves Phase 3 revert independence. OK.
- **`deprecated alias exports` for TS types in Phase 6**: `PromptTurnRow = PromptRequestRow`, `createSessionTurnsCollection = createSessionPromptRequestsCollection`. One-phase window before Phase 8 deletion. Standard migration shim — not a permanent API. OK.
- **`@fireline/state` ACP-id shim vs `@fireline/client` shim**: byte-for-byte identical files. Slight duplication, but eliminating it would couple the packages before Phase 6 finalizes. Acceptable interim state.
- **`AgentCatalog` vs deployment registry**: catalog names agent IDENTITIES; deployments are `DeploymentSpecSubscriber` materializations. Verified in hosted-deploy-surface-decision §4(d). OK.
- **`fireline.config.ts` vs `fireline.config` embedded in spec**: spec = portable, config = environment. `deployment-and-remote-handoff.md §4` holds the line. OK.
- **Sidecar vs bundled durable-streams**: production default is sidecar (independent lifecycle required by `SessionDurableAcrossRuntimeDeath`); bundled is quickstart-only with explicit durability warning. OK.
- **CI-first contention rule**: all dispatch prompts now assume CI is authoritative; no local cargo on shared worktree. Codified in `orchestration-status.md §Contention rules v2`. OK.

## 9. Final Alignment Call

**Aligned.** Proceed as scheduled:

- Dispatch Phase 3 once w17's TLC manual run confirms `ManagedAgentsCanonicalIds.cfg` invariants are non-vacuous.
- Keep Phases 3 → 3.5 → 4 tight on the calendar to minimize R3's lineage-gap window.
- DS Phase 6A (Tier C) stays deferred — no rush.
- All other tracks (OCI, CLI, OTel, Registry) continue in parallel — no agent-plane conflicts with canonical-ids sequence.

**No architectural pivots needed.** No vetoes pending. No open decisions blocking dispatch except R1 (TLC).

## References

- [acp-canonical-identifiers.md](../proposals/acp-canonical-identifiers.md)
- [acp-canonical-identifiers-execution.md](../proposals/acp-canonical-identifiers-execution.md) (patched `bfa222c`)
- [acp-canonical-identifiers-verification.md](../proposals/acp-canonical-identifiers-verification.md)
- [durable-subscriber.md](../proposals/durable-subscriber.md)
- [durable-subscriber-execution.md](../proposals/durable-subscriber-execution.md)
- [durable-subscriber-verification.md](../proposals/durable-subscriber-verification.md)
- [durable-promises.md](../proposals/durable-promises.md)
- [durable-promises-execution.md](../proposals/durable-promises-execution.md)
- [hosted-deploy-surface-decision.md](../proposals/hosted-deploy-surface-decision.md)
- [hosted-fireline-deployment.md](../proposals/hosted-fireline-deployment.md)
- [fireline-cli-execution.md](../proposals/fireline-cli-execution.md)
- [observability-integration.md](../proposals/observability-integration.md)
- [acp-registry-execution.md](../proposals/acp-registry-execution.md)
- [state-projector-surface-audit.md](../proposals/state-projector-surface-audit.md)
- [state-projector-audit-review-2026-04-12.md](./state-projector-audit-review-2026-04-12.md)
- [proposal-index.md](../proposals/proposal-index.md)
- [orchestration-status.md](../status/orchestration-status.md)
- [verification/spec/managed_agents.tla](../../verification/spec/managed_agents.tla)
