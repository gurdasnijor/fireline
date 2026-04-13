# Durable Promises Replay Invariants

> Status: Phase 4 invariant register
> Date: 2026-04-12
> Scope: Durable Promises Phase 4 replay integration invariants and concrete test shapes
> Gate: implemented by `tests/durable_promises_replay_skeleton.rs` and `packages/client/test/workflow-awakeable.integration.test.ts`

This document narrows Durable Promises Phase 4 from [durable-promises-execution.md](./durable-promises-execution.md) into five replay-specific proof targets.

Awakeables remain the imperative projection of `DurableSubscriber::Passive`; replay correctness must therefore refine the already-landed passive-subscriber behavior rather than introduce a second wait engine, a second resolver path, or process-local recovery state. The concrete reference behavior is the approval-gate passive replay path as ported in `4c5b207` (`crates/fireline-harness/src/approval.rs::rebuild_from_log`) and its restart/race coverage in `tests/managed_agent_harness.rs`.

## Working Rules

1. Replay implementation must stay a thin refinement of the passive-subscriber substrate. No parallel wait engine or replay-only cache is allowed.
2. The future Phase 4 PR must consume the passive-subscriber substrate already proven by DurableSubscriber and approval-gate replay.
3. Every replay test must prove convergence between live resolution and rebuild resolution. "Works after restart" is not enough if the replay path can double-subscribe or double-resolve.
4. No invariant here permits Fireline-minted awakeable ids, replay-only caches, or infra-plane leakage into the workflow API.

## Invariant Register

| ID | Invariant | Meaning | Substrate references |
|---|---|---|---|
| `DSV-dp-40 ReplayReturnsResolvedAwakeableWithoutResubscription` | Already-resolved awakeable returns resolved future on replay | If the matching completion envelope is already durable before rebuild, `ctx.awakeable(...).promise` resolves from replay without registering a fresh live waiter. | `DSV-02`, `DSV-aw-03`, approval `rebuild_from_log` |
| `DSV-dp-41 ReplayRebindsPendingAwakeableToLiveSubscriber` | Pending awakeable remains pending after replay | If replay sees a wait envelope but no completion, the rebuilt awakeable stays pending and binds to the live subscriber path for the eventual completion. | `DSV-10`, `DSV-aw-03` |
| `DSV-dp-42 RehydrationWindowResolutionDeliversExactlyOnce` | Resolution during replay wins exactly once | If the resolver fires while replay is reconstructing waits, the rebuilt workflow observes one completion and never drops or duplicates it across the replay boundary. | `DSV-13`, `DSV-aw-03`, approval rebuild-race test |
| `DSV-dp-43 ConcurrentReplayAndLiveResolveDoNotDoubleResolve` | Replay/live convergence is single-winner | If replay and a concurrent live resolve race on the same `CompletionKey`, the future completes once and only once; duplicate completion visibility is forbidden. | `DSV-01`, `DSV-02`, `DSV-12`, `DSV-13` |
| `DSV-dp-44 TraceparentPreservedAcrossReplayBoundary` | `_meta.traceparent` lineage survives replay | A replayed awakeable wait and its completion preserve the same W3C trace lineage the live path would expose; restart must not sever or rewrite trace continuity. | `DSV-05`, `DSV-aw-04`, `DSV-aw-06` |

## Scenario Shapes

### `DSV-dp-40 ReplayReturnsResolvedAwakeableWithoutResubscription`

**Scenario shape**
- Append an awakeable wait envelope keyed by canonical `CompletionKey`.
- Append the matching completion envelope before runtime shutdown.
- Restart, replay the stream, and re-enter workflow context for the same logical step.

**Expected assertions**
- The replayed `ctx.awakeable(...).promise` resolves immediately.
- No second passive registration or second live subscription is emitted for the same key.
- The completion value matches the already-durable completion envelope.

**Failure this catches**
- Rebuild code that always creates a fresh pending waiter even when replay has already seen the winner.

### `DSV-dp-41 ReplayRebindsPendingAwakeableToLiveSubscriber`

**Scenario shape**
- Append an awakeable wait envelope without its completion.
- Restart before any resolver fires.
- Replay, reconstruct workflow context, then append the matching completion envelope after rebuild finishes.

**Expected assertions**
- The replayed `ctx.awakeable(...).promise` remains pending until the new completion arrives.
- The eventual completion is delivered through the rebuilt live subscriber path, not by replaying stale in-memory state.
- Exactly one logical waiter is released when the completion arrives.

**Failure this catches**
- Rebuild code that drops unresolved waits or reconstructs them in a detached state that never rejoins the live subscriber path.

### `DSV-dp-42 RehydrationWindowResolutionDeliversExactlyOnce`

**Scenario shape**
- Start replay of a session with an unresolved awakeable wait.
- While replay is between "wait discovered" and "rebuild complete", append the matching completion envelope.
- Let replay finish and observe the resumed workflow.

**Expected assertions**
- The resumed workflow completes the awakeable exactly once.
- The rebuilt state converges on the same single winner regardless of whether replay or the live append is observed first.
- No second completion event, second user-visible callback, or second state transition is emitted.

**Failure this catches**
- The exact rebuild-race class already exercised in approval-gate replay, but reintroduced in the awakeable path.

### `DSV-dp-43 ConcurrentReplayAndLiveResolveDoNotDoubleResolve`

**Scenario shape**
- Register two replay-sensitive observers for the same awakeable key: one through replay reconstruction and one through the live runtime after restart.
- Race a matching completion append against replay finishing.

**Expected assertions**
- Both observers converge on one semantic completion winner.
- Duplicate append attempts are semantic no-ops.
- No double-resolution callback, duplicate promise fulfillment, or divergent final state is observable.

**Failure this catches**
- Any implementation that treats replay reconstruction and live subscription as separate completion domains.

### `DSV-dp-44 TraceparentPreservedAcrossReplayBoundary`

**Scenario shape**
- Emit an awakeable wait envelope with `_meta.traceparent` populated.
- Restart across replay, then resolve the awakeable and inspect the resumed completion path.

**Expected assertions**
- The completion path preserves the original trace lineage across replay.
- Replay does not replace `_meta.traceparent` with a workflow-local correlation id, subscriber-private token, or restart-local lineage.
- The resumed workflow observes the same trace continuity it would have seen without a restart.

**Failure this catches**
- Replay code that reconstructs waits semantically but loses observability lineage.

## Test Mapping

The current Phase 4 tests map 1:1 to the invariant IDs above:

| Test function | Invariant |
|---|---|
| `replay_returns_resolved_awakeable_without_resubscription` | `DSV-dp-40` |
| `replay_rebinds_pending_awakeable_to_live_subscriber` | `DSV-dp-41` |
| `resolution_during_rehydration_window_is_delivered_exactly_once` | `DSV-dp-42` |
| `concurrent_replay_and_live_resolve_do_not_double_resolve` | `DSV-dp-43` |
| `traceparent_is_preserved_across_replay_boundary` | `DSV-dp-44` |

## Validation Checklist

- [x] Every Phase 4 replay test cites one of `DSV-dp-40` through `DSV-dp-44`.
- [x] Replay assertions are phrased as live-vs-replay convergence, not restart-only happy path.
- [x] At least one test shape explicitly mirrors the approval rebuild-race proof already carried by `harness_approval_resolution_during_rebuild_marks_session_approved`.
- [x] `_meta.traceparent` is asserted as part of replay correctness, not deferred to a later observability sweep.
- [x] No test assumes a process-local promise table, replay-only cache, or non-canonical awakeable id.

## Architect Review Checklist

- [ ] The five `DSV-dp-*` invariants refine the existing DurableSubscriber and awakeable invariants rather than creating a second proof surface.
- [ ] The scenario shapes cover both replay-found completion and replay-missing completion cases.
- [ ] Rebuild-race and double-resolution are treated as distinct hazards, not collapsed into one vague "resume works" claim.
- [ ] Trace lineage preservation is included in replay semantics, not treated as optional polish.
- [ ] The landed tests remain aligned with these invariants as the awakeable surface evolves.

## References

- [durable-promises.md](./durable-promises.md)
- [durable-promises-execution.md](./durable-promises-execution.md)
- [durable-subscriber-verification.md](./durable-subscriber-verification.md)
- [approval.rs](../../crates/fireline-harness/src/approval.rs)
- [managed_agent_harness.rs](../../tests/managed_agent_harness.rs)
