# Durable Subscriber Execution Plan

> Status: execution plan
> Date: 2026-04-12
> Scope: Rust subscriber substrate, approval refactor, active subscriber profiles, TypeScript middleware surface
> Blocker: implementation Phases 1-8 are blocked on [acp-canonical-identifiers-execution.md Phase 5](./acp-canonical-identifiers-execution.md#phase-5-delete-activeturnindex-and-child_session_edge). Phase 0 may land earlier because it is docs-only.

This document is the rollout plan for [durable-subscriber.md](./durable-subscriber.md). It is intentionally operational: each phase is small enough to land on `main`, must be revertable on its own, and uses CI as the binding gate per the current shared-worktree contention rules.

The execution order assumes the canonical identifier contract is already in force. DurableSubscriber is not allowed to freeze transitional seams such as `ActiveTurnIndex`, `child_session_edge`, or synthetic completion ids into its public or internal contract.

## Working Rules

1. Land directly on `main` as short-lived PRs. Do not build a long-lived subscriber branch.
2. One phase per PR. Do not mix trait extraction, approval migration, and new subscriber profiles in the same rollout slice.
3. CI-first only. Per the v2 contention rules in [docs/status/orchestration-status.md](../status/orchestration-status.md), use GitHub Actions as the sole binding gate for code phases. Do not treat local cargo runs as authoritative.
4. Phase 0 is the only allowed pre-blocker phase. It is docs-only and exists to give the later phases stable invariant IDs. Phases 1-8 do not start until canonical-identifiers Phase 5 is green on `main`.
5. Preserve behavior during migration. The approval gate remains the semantic reference implementation until its trait-backed replacement proves the same durability, replay, timeout, and concurrent-isolation guarantees.
6. Subscriber bookkeeping stays in the infrastructure plane. Agent-plane completions, keys, and trace context must remain canonical ACP-shaped throughout the rollout.

## Compatibility Strategy

Use an additive, profile-by-profile migration:

- Phase 1 adds the Rust substrate and registration points without changing behavior.
- Phase 2 ports the existing approval gate onto that substrate while keeping the current approval semantics and tests intact.
- Phases 3-6 add new subscriber profiles one at a time, all opt-in and configuration-gated.
- Phase 7 exposes the TypeScript helper only after the Rust substrate and the first subscriber profiles are stable.
- Phase 8 removes temporary adapters and migration shims only after all earlier phases are green and the verification doc confirms the steady-state surface.

This keeps rollback small and avoids dual-migrating agent-plane identity at the same time as subscriber behavior.

## Phase 0: Verification Doc Prerequisite

**Invariant mapping**
- `DS-TBD-00 VerificationPlanExists` in `docs/proposals/durable-subscriber-verification.md` (sibling task on w22; placeholder until that doc lands)

**Scope**
- Create `docs/proposals/durable-subscriber-verification.md`.
- Lift the approval-gate proof obligations from [approval-gate-correctness.md](../reviews/approval-gate-correctness.md) into stable DurableSubscriber invariant IDs.
- Record the required dependency on canonical-identifiers Phase 5 and the cross-reference to [durable-promises.md](./durable-promises.md).

**Preconditions**
- None. This is the only phase allowed before canonical-identifiers Phase 5.

**Gate command list (CI)**
```bash
gh pr checks --watch
gh run list --limit 5 --json databaseId,displayTitle,status,conclusion,url
gh run watch <databaseId>
```
Required green checks: docs/links only, plus any existing proposal-doc validation jobs.

**Risks**
- If invariant IDs are vague here, later phases will cite unstable proof targets and the rollout will drift.

**Done when**
- `durable-subscriber-verification.md` exists and assigns stable invariant IDs.
- Each later phase in this doc can point to a verification target without inventing new wording ad hoc.
- The blocker relationship to canonical-identifiers Phase 5 is explicit in both docs.

**Rollback**
- Revert the docs-only PR.

## Phase 1: Rust Trait Surface

**Invariant mapping**
- `DS-TBD-01 NoSyntheticCompletionKeys`
- `DS-TBD-02 PassiveAndActiveShareOneSubstrate`
- `DS-TBD-03 RegistrationDoesNotCrossPlanes`

**Scope**
- Add the Rust substrate described in [durable-subscriber.md §4](./durable-subscriber.md#4-rust-design):
  - `DurableSubscriber`
  - `ActiveSubscriber`
  - `PassiveSubscriber`
  - `CompletionKey`
  - subscriber registration and driver bootstrap wiring
- Keep the driver skeleton inert until Phase 2 ports the first real consumer.
- Ensure `CompletionKey` only admits canonical ACP-shaped variants.

**Preconditions**
- Phase 0 complete.
- [acp-canonical-identifiers-execution.md Phase 5](./acp-canonical-identifiers-execution.md#phase-5-delete-activeturnindex-and-child_session_edge) landed and green.

**Gate command list (CI)**
```bash
gh pr checks --watch
gh run list --limit 5 --json databaseId,displayTitle,status,conclusion,url
gh run watch <databaseId>
```
Required green checks: Rust workspace build/test jobs for the new substrate and any subscriber-core unit tests introduced in the phase PR.

**Risks**
- Over-designing the trait around non-canonical ids or subscriber-specific quirks.
- Baking approval-specific semantics into the generic surface.
- Accidentally leaking infrastructure bookkeeping onto agent-plane types.

**Done when**
- The Rust trait and registration surface exists behind stable module exports.
- `CompletionKey` has only canonical ACP-bound variants.
- The driver can register subscribers without changing approval behavior yet.

**Rollback**
- Revert the trait-surface PR only.

## Phase 2: Refactor Approval Gate onto the Trait

**Invariant mapping**
- `DS-TBD-10 ApprovalReplayResumesExactlyOnce`
- `DS-TBD-11 ApprovalTimeoutIsObservableAndBounded`
- `DS-TBD-12 ConcurrentApprovalIsolationByCanonicalKey`

**Scope**
- Refactor [crates/fireline-harness/src/approval.rs](../../crates/fireline-harness/src/approval.rs) so the approval gate becomes the first `PassiveSubscriber`.
- Keep the existing approval semantics as the reference behavior:
  - durable suspend/resume
  - restart-safe replay
  - timeout behavior
  - concurrent isolation
  - rebuild-race safety
- Preserve the externally visible approval flow so the rest of the harness does not change at the same time.

**Preconditions**
- Phase 1 complete.
- Canonical ACP identifiers are already the approval key surface; no approval fallback path should depend on deleted synthetic lineage.

**Gate command list (CI)**
```bash
gh pr checks --watch
gh run list --limit 5 --json databaseId,displayTitle,status,conclusion,url
gh run watch <databaseId>
```
Required green checks: Rust workspace, `fireline-harness` tests, and the approval-focused harness/component tests covered by the reference review.

**Risks**
- Regressing the only proven durable wait path while extracting the trait.
- Preserving semantics incompletely, especially around rebuild and timeout.
- Leaving duplicated approval logic in both the old and new paths.

**Done when**
- `approval.rs` uses the trait-backed subscriber driver rather than bespoke wait plumbing.
- The approval gate remains the reference consumer and still satisfies the proof points from [approval-gate-correctness.md](../reviews/approval-gate-correctness.md).
- No behavior-only shim remains except the minimal adapter needed for phase-local rollback.

**Rollback**
- Revert the approval-refactor PR only.

## Phase 3: Webhook Delivery Subscriber

**Invariant mapping**
- `DS-TBD-20 WebhookDeliveryIsAtLeastOnce`
- `DS-TBD-21 WebhookCursorAdvancesOnlyAfterAck`
- `DS-TBD-22 WebhookTraceContextPropagates`

**Scope**
- Implement the webhook delivery profile from [durable-subscriber.md §5.2](./durable-subscriber.md#52-webhooksubscriber).
- Support host-owned webhook target config, cursor persistence, retry policy, and dead-letter bookkeeping in the infrastructure plane.
- Write `webhook_delivered` completions back to the agent stream keyed by canonical `PromptKey` or `ToolKey`.

**Preconditions**
- Phase 2 complete.
- The driver already supports active subscribers and infra-plane bookkeeping.

**Gate command list (CI)**
```bash
gh pr checks --watch
gh run list --limit 5 --json databaseId,displayTitle,status,conclusion,url
gh run watch <databaseId>
```
Required green checks: Rust workspace, subscriber/webhook integration tests, and any HTTP delivery fixture tests added in the phase PR.

**Risks**
- Confusing infrastructure cursor state with agent-plane completion state.
- Advancing the cursor before a durable acknowledgment.
- Failing to propagate W3C trace headers or `_meta` correctly.

**Done when**
- `WebhookSubscriber` exists as an active subscriber profile.
- Cursor persistence, retry, and dead-letter behavior live only in infra streams.
- Delivery completions are appended back to the agent stream with canonical keys and copied trace context.

**Rollback**
- Revert the webhook-subscriber PR only.

## Phase 4: Auto-Approve Subscriber

**Invariant mapping**
- `DS-TBD-30 AutoApproveWritesCanonicalApprovalResolution`
- `DS-TBD-31 PassiveAndActiveApprovalPathsInteroperate`

**Scope**
- Add an active auto-approval subscriber that watches `permission_request` and resolves `approval_resolved` according to policy.
- Reuse the same canonical key path and completion envelope as the passive approval gate.
- Keep policy decisions host-owned; do not serialize secrets or mutable policy engines into agent specs.

**Preconditions**
- Phase 2 complete.
- Approval gate behavior is already stable on the generic substrate.

**Gate command list (CI)**
```bash
gh pr checks --watch
gh run list --limit 5 --json databaseId,displayTitle,status,conclusion,url
gh run watch <databaseId>
```
Required green checks: Rust workspace plus targeted approval/auto-approve tests proving passive and active paths can coexist safely.

**Risks**
- Inventing a second approval identity path.
- Letting auto-approve bypass the canonical completion envelope.
- Blurring policy config with agent-plane data.

**Done when**
- Auto-approve resolves the same `approval_resolved` envelope shape as the passive gate.
- Policy-driven approval remains additive and opt-in.
- The approval gate and auto-approve subscriber can coexist without ambiguity in completion matching.

**Rollback**
- Revert the auto-approve PR only.

## Phase 5: Peer-Routing Subscriber

**Invariant mapping**
- `DS-TBD-40 PeerDeliveryCompletionIsCallerLocal`
- `DS-TBD-41 CrossSessionLineageFlowsOnlyThroughTraceContext`
- `DS-TBD-42 PeerSubscriberDoesNotReintroduceChildEdges`

**Scope**
- Add the peer-routing subscriber profile described in [durable-subscriber.md §5.4](./durable-subscriber.md#54-peercallsubscriber).
- Use canonical caller-local completion keys:
  - `PromptKey(SessionId, RequestId)`
  - `ToolKey(SessionId, ToolCallId)`
- Propagate `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage` across outbound and inbound peer delivery.

**Preconditions**
- Phase 2 complete.
- Canonical-identifiers Phase 5 already landed; no code in this phase may depend on `ActiveTurnIndex` or `child_session_edge`.
- Peer transport is already on the canonical trace-context path.

**Gate command list (CI)**
```bash
gh pr checks --watch
gh run list --limit 5 --json databaseId,displayTitle,status,conclusion,url
gh run watch <databaseId>
```
Required green checks: Rust workspace and peer-routing integration tests proving caller-local completion plus trace propagation.

**Risks**
- Reintroducing bespoke cross-session lineage under a new subscriber name.
- Confusing callee session identity with caller-side completion identity.
- Overlapping peer transport refactors with subscriber rollout.

**Done when**
- Peer routing completes back onto the caller stream using canonical caller-local keys only.
- No lineage field or edge table is added back.
- Trace context, not bespoke ids, is the only cross-session linkage.

**Rollback**
- Revert the peer-routing subscriber PR only.

## Phase 6: Wake and Deployment Subscribers

**Invariant mapping**
- `DS-TBD-50 AgentBoundTimerFiresAtMostOncePerKey`
- `DS-TBD-51 ReplayRestoresPendingTimerWaits`
- `WakeOnReadyIsNoop` from `verification/spec/managed_agents.tla`
- `WakeOnStoppedChangesRuntimeId` from `verification/spec/managed_agents.tla`
- `WakeOnStoppedPreservesSessionBinding` from `verification/spec/managed_agents.tla`
- `ConcurrentWakeSingleWinner` from `verification/spec/managed_agents.tla`
- `SessionDurableAcrossRuntimeDeath` from `verification/spec/managed_agents.tla`

**Scope**
- Add the agent-bound timer subscriber from [durable-subscriber.md §5.5](./durable-subscriber.md#55-waketimersubscriber).
- Limit the first cut to agent-bound timers keyed by `PromptKey(SessionId, RequestId)`.
- Add `AlwaysOnDeploymentSubscriber` as an active subscriber profile for the always-on sandbox policy:
  - event: `deployment_wake_requested`
  - key: `SessionId` (deployment identity)
  - completion: `sandbox_provisioned` (with runtime id)
  - mode: active
- Treat `AlwaysOnDeploymentSubscriber` as glue over the existing wake/provision path, not as a new semantic primitive. The subscriber translates boot-time scan or heartbeat wake requests into the already-modeled wake state machine.
- Explicitly defer infrastructure-only cluster timers until a separate infra-only contract exists.

**Preconditions**
- Phase 1 complete.
- Prefer Phase 2 complete as well, so passive subscriber behavior is already exercised in production by approvals before timer waits are introduced.

**Gate command list (CI)**
```bash
gh pr checks --watch
gh run list --limit 5 --json databaseId,displayTitle,status,conclusion,url
gh run watch <databaseId>
```
Required green checks: Rust workspace plus wake/deployment replay, resume, and single-winner tests added in the phase PR.

**Risks**
- Accidentally mixing infra-only scheduler state with agent-bound completions.
- Double-firing after restart if cursor/replay semantics are wrong.
- Turning timers into a second workflow substrate instead of another subscriber profile.
- Re-implementing the wake state machine inside the subscriber instead of delegating to the existing provision/wake path.

**Done when**
- Agent-bound timers append `timer_fired` completions keyed by canonical ACP identifiers.
- Replay restores unresolved waits without double-firing.
- `AlwaysOnDeploymentSubscriber` drives `lifecycle.alwaysOn = true` deployments through `deployment_wake_requested -> sandbox_provisioned` using the existing wake/provision substrate and the listed TLA wake invariants as its semantic base.
- Infrastructure-only timers remain out of scope and undocumented as supported behavior.

**Rollback**
- Revert the wake/deployment-subscriber PR only.

## Phase 7: TypeScript Middleware Surface

**Invariant mapping**
- `DS-TBD-60 MiddlewareLowersToCanonicalSubscriberConfig`
- `DS-TBD-61 TSAPIAcceptsNoSyntheticCompletionKeys`
- `DS-TBD-62 DurablePromisesCompanionSurfaceRemainsCompatible`

**Scope**
- Add `durableSubscriber()` to `@fireline/client` and export the relevant config/types.
- Ensure the TS lowering matches the Rust registration surface and leaves room for the imperative companion API proposed in [durable-promises.md](./durable-promises.md).
- Keep this phase declarative only: no `ctx.awakeable()` here, just the middleware/config surface the Rust substrate already supports.

**Preconditions**
- Phases 1-6 complete for the profiles that will be exposed publicly.
- The Rust config surface is stable enough that the TS helper is not forced into an immediate breaking rename.

**Gate command list (CI)**
```bash
gh pr checks --watch
gh run list --limit 5 --json databaseId,displayTitle,status,conclusion,url
gh run watch <databaseId>
```
Required green checks: `@fireline/client` build/test jobs plus any subscriber-surface examples or type tests added in the phase PR.

**Risks**
- Shipping a TS surface that does not map cleanly to the Rust substrate.
- Exposing raw strings or bespoke keys where canonical ACP-typed strategies should be required.
- Accidentally coupling the durable-promises API to a still-moving middleware surface.

**Done when**
- `durableSubscriber()` is exported from `@fireline/client`.
- Its config surface lowers to the Rust substrate without user-supplied synthetic keys.
- The API shape remains compatible with the future passive-subscriber sugar in [durable-promises.md](./durable-promises.md).

**Rollback**
- Revert the TS middleware PR only.

## Phase 8: Cleanup and Shim Removal

**Invariant mapping**
- `DS-TBD-70 NoLegacySubscriberShimRemains`
- `DS-TBD-71 OnlyCanonicalProfilesAndKeysRemain`

**Scope**
- Remove migration adapters introduced during Phases 1-7.
- Delete any temporary approval wrappers or registration shims kept only for rollback safety.
- Tighten docs and exports so the steady-state DurableSubscriber surface is the only supported path.

**Preconditions**
- Phases 1-7 complete and green.
- `durable-subscriber-verification.md` confirms the steady-state regression suite is in place.

**Gate command list (CI)**
```bash
gh pr checks --watch
gh run list --limit 5 --json databaseId,displayTitle,status,conclusion,url
gh run watch <databaseId>
```
Required green checks: full required Rust/TS CI for the touched packages and any new subscriber regression suites introduced earlier in the rollout.

**Risks**
- Deleting rollback hooks too early.
- Leaving one profile on a legacy path while docs claim the migration is finished.
- Removing compatibility shims before the verification doc proves equivalent coverage.

**Done when**
- No temporary DurableSubscriber migration adapter remains in Rust or TypeScript.
- Approval, webhook, auto-approve, peer routing, and timer subscribers all run on the same steady-state substrate.
- Docs describe the steady-state surface only, not the rollout scaffolding.

**Rollback**
- Revert the cleanup PR only, provided the earlier phase PRs remain intact.

## Validation Checklist

Cross-check this list against `docs/proposals/durable-subscriber-verification.md` once w22 lands it:

- [ ] Every phase above is mapped to one or more invariant IDs in `durable-subscriber-verification.md`.
- [ ] Phase 2 preserves the approval-gate guarantees already proved in [approval-gate-correctness.md](../reviews/approval-gate-correctness.md).
- [ ] Phase 3 proves at-least-once webhook delivery, cursor monotonicity, and trace propagation.
- [ ] Phase 4 proves auto-approve emits the same canonical completion envelope as the passive approval path.
- [ ] Phase 5 proves peer completion remains caller-local and cross-session lineage remains trace-only.
- [ ] Phase 6 proves agent-bound timers replay safely and do not double-fire.
- [ ] Phase 6 proves `AlwaysOnDeploymentSubscriber` relies on the existing wake invariants rather than introducing a second deployment-lifecycle primitive.
- [ ] Phase 7 proves the TS helper exposes only canonical key strategies and stays compatible with [durable-promises.md](./durable-promises.md).
- [ ] Phase 8 proves no migration shim or legacy completion-key surface remains.

## Pre-Dispatch Checklist

Architect should confirm all of the following before execution starts:

- [ ] Phase 0 is assigned or landed so the `DS-TBD-*` placeholders can be replaced with real invariant IDs.
- [ ] The blocker is honored: Phases 1-8 do not start before canonical-identifiers Phase 5 is green on `main`.
- [ ] Each phase boundary is clean enough to revert independently without reverting another phase.
- [ ] The approval gate remains the reference consumer until Phase 2 is complete and verified.
- [ ] Peer routing in Phase 5 does not depend on deleted lineage structures or reintroduce them indirectly.
- [ ] Wake timers in Phase 6 are explicitly limited to agent-bound timers; infra-only timers remain deferred.
- [ ] `AlwaysOnDeploymentSubscriber` is treated as a DurableSubscriber profile over the existing wake/provision path, not as a separate primitive.
- [ ] The Phase 7 TS surface is narrow enough to stay compatible with the future imperative API in [durable-promises.md](./durable-promises.md).
- [ ] The CI gates named in each phase are feasible with existing GitHub Actions or are explicitly added in the same PR that needs them.

## References

- [durable-subscriber.md](./durable-subscriber.md)
- [durable-promises.md](./durable-promises.md)
- [acp-canonical-identifiers.md](./acp-canonical-identifiers.md)
- [acp-canonical-identifiers-execution.md](./acp-canonical-identifiers-execution.md)
- [approval-gate-correctness.md](../reviews/approval-gate-correctness.md)
- [crates/fireline-harness/src/approval.rs](../../crates/fireline-harness/src/approval.rs)
