# Durable Promises Execution Plan

> Status: execution plan
> Date: 2026-04-12
> Scope: Rust workflow-context sugar, awakeable resolution API, TypeScript workflow surface, replay integration
> Blocker: implementation Phases 1-5 are blocked on [durable-subscriber-execution.md Phase 7](./durable-subscriber-execution.md#phase-7-typescript-middleware-surface). Phase 0 may land earlier because it is docs-only.

This document is the rollout plan for [durable-promises.md](./durable-promises.md). It is intentionally operational: each phase is small enough to land on `main`, revertable on its own, and gated by CI per the shared-worktree v2 rules.

Awakeable is not a new substrate. It is the imperative projection of `DurableSubscriber::Passive`, layered on top of the subscriber substrate and canonical ACP identity contract that are already defined elsewhere. This plan therefore covers only the imperative API, replay binding, and verification additions needed to expose that substrate safely to workflow authors.

**BLOCKED ON DurableSubscriber Phase 7 landing.** Do not start implementation Phases 1-5 until [durable-subscriber-execution.md Phase 7](./durable-subscriber-execution.md#phase-7-typescript-middleware-surface) is green on `main`. The imperative layer must consume the landed subscriber substrate and TypeScript lowering surface; it must not race them or fork them.

## Working Rules

1. Land directly on `main` as short-lived PRs. Do not build a long-lived awakeable branch.
2. One phase per PR. Do not mix Rust workflow sugar, resolver plumbing, replay, and TypeScript ergonomics in one slice.
3. CI-first only. Per the v2 contention rules in [docs/status/orchestration-status.md](../status/orchestration-status.md), use GitHub Actions as the sole binding gate for code phases.
4. Phase 0 is the only allowed pre-blocker phase. It is docs-only. Phases 1-5 do not start until DurableSubscriber Phase 7 is landed and green.
5. Do not duplicate DurableSubscriber trait or driver design here. Every phase must consume the existing passive-subscriber substrate rather than re-specifying it.
6. Preserve plane separation from [acp-canonical-identifiers.md](./acp-canonical-identifiers.md). Awakeable keys, waits, and completions stay in the agent plane and use canonical ACP identifiers only. Subscriber cursor, retry, and dead-letter bookkeeping remain in the infrastructure plane and never surface in the imperative API or examples.

## Compatibility Strategy

Use an additive, thin-wrapper rollout:

- Phase 1 adds Rust workflow-context sugar over the landed passive-subscriber path.
- Phase 2 adds canonical resolver helpers without changing the completion-envelope contract.
- Phase 3 exposes the TypeScript workflow surface only after the Rust and substrate seams are stable.
- Phase 4 wires replay so unresolved waits are reconstructed from the durable stream alone.
- Phase 5 is optional ergonomic polish and must remain additive.

This keeps rollback small and prevents the imperative layer from inventing a second wait engine, a second identifier scheme, or hidden replay state.

## Phase 0: Verification Alignment

**Invariant mapping**
- `DP-TBD-00 AwakeableVerificationSectionExists`
- `DP-TBD-01 ReplayRebuildsOutstandingAwakeables`
- `DP-TBD-02 ResolutionUsesCanonicalCompletionKey`
- `DP-TBD-03 AwakeableSurfacePreservesPlaneBoundary`

**Scope**
- Add an `Awakeable` section to `docs/proposals/durable-subscriber-verification.md`.
- If that sibling verification doc is still not on `main`, couple this phase to [durable-subscriber-execution.md Phase 0](./durable-subscriber-execution.md#phase-0-verification-doc-prerequisite) so the file is created first and the awakeable section lands immediately after.
- Map the validation checklist in [durable-promises.md §7](./durable-promises.md#7-acceptance-criterion-alignment) to stable invariant IDs.
- Record that awakeable correctness is passive-subscriber correctness plus API-surface obligations, not a second proof domain.

**Preconditions**
- None for the plan doc itself.
- If `durable-subscriber-verification.md` does not yet exist, DurableSubscriber Phase 0 must land first or this phase must be bundled as an explicit extension to that docs-only prerequisite.

**Gate command list (CI)**
```bash
gh pr checks --watch
gh run list --limit 5 --json databaseId,displayTitle,status,conclusion,url
gh run watch <databaseId>
```
Required green checks: docs/links validation and any existing proposal-doc lint jobs.

**Risks**
- If the awakeable invariants are vague, later phases will drift into re-specifying substrate semantics or skipping replay proof obligations.
- If the verification section is not anchored in the subscriber verification doc, the imperative layer will accumulate a parallel checklist with inconsistent wording.

**Done when**
- `durable-subscriber-verification.md` contains a stable `Awakeable` section with named invariant IDs.
- Every later phase in this document points to one or more of those invariant IDs.
- The blocker relationship to DurableSubscriber Phase 7 is explicit in both execution plans.

**Rollback**
- Revert the docs-only PR.

## Phase 1: Rust Workflow Context Surface

**Invariant mapping**
- `DP-TBD-10 RustAwakeableIsThinPassiveProjection`
- `DP-TBD-11 RustAwakeableAcceptsCanonicalCompletionKeyOnly`

**Scope**
- Add the Rust workflow-context surface described in [durable-promises.md §3](./durable-promises.md#3-the-api):
  - `ctx.awakeable<T>(key)` or equivalent workflow-context entry point
  - returned value is a subscriber-backed future/await handle over `DurableSubscriber::Passive`
- Reuse the existing completion-envelope contract and `CompletionKey` type from the subscriber substrate.
- Keep this phase Rust-only. Do not expose TypeScript sugar or replay-specific convenience helpers yet.

**Preconditions**
- Phase 0 complete.
- [durable-subscriber-execution.md Phase 7](./durable-subscriber-execution.md#phase-7-typescript-middleware-surface) landed and green.

**Gate command list (CI)**
```bash
gh pr checks --watch
gh run list --limit 5 --json databaseId,displayTitle,status,conclusion,url
gh run watch <databaseId>
```
Required green checks: Rust workspace build/test jobs and any workflow-context unit tests added in the phase PR.

**Risks**
- Reintroducing a second wait abstraction instead of calling through the landed passive-subscriber path.
- Accepting raw strings or Fireline-minted ids at the Rust API seam.
- Hiding infrastructure subscriber state behind the workflow handle.

**Done when**
- Rust workflow code can declare an awakeable wait keyed by canonical `CompletionKey`.
- The returned future is visibly backed by passive-subscriber semantics rather than bespoke promise storage.
- No new awakeable-specific id type, queue, or driver is introduced.

**Rollback**
- Revert the Rust workflow-surface PR only.

## Phase 2: Canonical `resolveAwakeable()` API

**Invariant mapping**
- `DP-TBD-20 ResolverAppendsCanonicalCompletionEnvelope`
- `DP-TBD-21 ApprovalResolverRemainsSameMechanism`

**Scope**
- Add `resolveAwakeable(key, value)` as the generic imperative resolver in both Rust- and TypeScript-facing layers.
- Keep `agent.resolvePermission()` as the approval-specific convenience surface, but route it through the same completion-envelope path.
- Ensure the resolver accepts canonical `CompletionKey` variants only and does not mint synthetic completion ids.

**Preconditions**
- Phase 1 complete.
- DurableSubscriber passive completion append path is already stable on `main`.

**Gate command list (CI)**
```bash
gh pr checks --watch
gh run list --limit 5 --json databaseId,displayTitle,status,conclusion,url
gh run watch <databaseId>
```
Required green checks: Rust workspace, `@fireline/client` build/test jobs, and resolver-focused unit or integration coverage added in the phase PR.

**Risks**
- Creating a second resolution path that diverges from the subscriber completion envelope.
- Allowing caller-supplied raw strings or opaque ids to bypass canonical `CompletionKey`.
- Breaking the approval-specific convenience path while generalizing it.

**Done when**
- `resolveAwakeable()` exists as the generic completion API and appends the same durable completion envelope passive subscribers already consume.
- Approval-specific resolution is clearly documented as a thin specialization of the same mechanism.
- No resolver API accepts a synthetic UUID, hash, or infrastructure identifier.

**Rollback**
- Revert the resolver PR only.

## Phase 3: TypeScript Workflow Surface

**Invariant mapping**
- `DP-TBD-30 TSSurfaceUsesCanonicalACPTypes`
- `DP-TBD-31 TSSurfacePreservesPlaneBoundary`
- `DP-TBD-32 ExamplesStayImperativeNotInfraLeaky`

**Scope**
- Add the TypeScript workflow surface from [durable-promises.md §3](./durable-promises.md#3-the-api):
  - `WorkflowContext.awakeable<T>(...)`
  - `Awakeable<T>`
  - any typed helper exports needed to construct prompt/tool scopes without raw strings
- Export the relevant types from `@fireline/client`.
- Keep examples and docs visibly agent-plane only: `SessionId`, `RequestId`, `ToolCallId`, `CompletionKey`, and `PromptStepKey` where applicable. No cursor streams, retry state, host ids, or other infra-plane concepts appear in user-facing examples.

**Preconditions**
- Phase 2 complete.
- DurableSubscriber Phase 7 is already landed, so the TypeScript middleware/config surface is stable enough to layer the imperative API on top without immediate churn.

**Gate command list (CI)**
```bash
gh pr checks --watch
gh run list --limit 5 --json databaseId,displayTitle,status,conclusion,url
gh run watch <databaseId>
```
Required green checks: `@fireline/client` build/test jobs, any TypeScript type fixtures added for awakeable APIs, and proposal/example validation jobs if examples are touched.

**Risks**
- Surfacing plain `string` instead of ACP SDK identifier types.
- Letting userland construct non-canonical or infra-leaky keys.
- Exposing a TypeScript API that implies a second runtime distinct from the subscriber substrate.

**Done when**
- `@fireline/client` exposes the minimal imperative awakeable surface described in the proposal.
- Prompt and tool scope helpers type through ACP SDK identifiers.
- Examples preserve the plane split visibly and do not reference subscriber-internal bookkeeping.

**Rollback**
- Revert the TypeScript workflow-surface PR only.

## Phase 4: Replay Integration

**Invariant mapping**
- `DP-TBD-40 ReplayReconstructsUnresolvedWaits`
- `DP-TBD-41 StepAwakeablesKeyByStreamOffset`
- `DP-TBD-42 NoHiddenReplayStateOutsideStream`

**Scope**
- Wire replay so the runtime reconstructs suspended awakeable waits from the durable stream after restart or rebuild.
- When the stream projector or replay path encounters an unresolved awakeable wait envelope, surface the suspended wait again rather than dropping it.
- Resolve immediately on replay when the matching completion envelope is already present.
- Ensure step-scoped awakeables derive `PromptStepKey(SessionId, RequestId, StreamOffset)` from the wait event's durable offset rather than a Fireline counter or random token.

**Preconditions**
- Phase 3 complete.
- The passive-subscriber substrate and imperative resolver are already stable enough that replay is binding the same key/completion semantics, not translating them.

**Gate command list (CI)**
```bash
gh pr checks --watch
gh run list --limit 5 --json databaseId,displayTitle,status,conclusion,url
gh run watch <databaseId>
```
Required green checks: Rust workspace, harness/workflow replay tests, restart/rebuild regression coverage, and any cross-language awakeable integration tests added in the phase PR.

**Risks**
- Losing suspended waits on replay because the runtime depends on process-local promise state.
- Double-resolving or double-registering waits after restart.
- Deriving step keys from anything other than canonical ACP ids plus durable stream offset.

**Done when**
- Restart and replay reconstruct unresolved awakeables from the stream alone.
- The same completion envelope resolves the same wait both live and during rebuild.
- No sidecar table or in-memory-only identifier is required to restore suspended waits.

**Rollback**
- Revert the replay-integration PR only.

## Phase 5: Optional Ergonomic Sugar

**Invariant mapping**
- `DP-TBD-50 ErgonomicSugarRemainsAdditive`
- `DP-TBD-51 PromiseLikeSugarDoesNotHideCanonicalKey`

**Scope**
- Add only additive ergonomics after Phases 1-4 are stable.
- Candidate sugar includes:
  - `PromiseLike` or similar awaitable-style polish layered on top of the Phase 3 `awakeable.promise` surface
  - small helper overloads for common prompt/tool usage that still preserve ACP-typed inputs
- Do not let this phase redefine replay semantics, completion keys, or subscriber behavior.

**Preconditions**
- Phase 4 complete.
- The base awakeable surface is already proven in replay and restart tests.

**Gate command list (CI)**
```bash
gh pr checks --watch
gh run list --limit 5 --json databaseId,displayTitle,status,conclusion,url
gh run watch <databaseId>
```
Required green checks: `@fireline/client` build/test jobs plus any ergonomics-focused type fixtures or workflow examples introduced in the phase PR.

**Risks**
- Hiding the canonical key so deeply that debugging and external resolution become opaque.
- Reopening naming churn after the core surface is already in use.
- Slipping non-additive behavior changes into what should be a polish-only phase.

**Done when**
- Any ergonomic additions are strictly additive and keep `CompletionKey` visible.
- Workflow authors can compose awakeables naturally with `Promise.race` and `Promise.all` without new substrate semantics.
- Skipping this phase would still leave a correct and supportable imperative surface.

**Rollback**
- Revert the ergonomics-only PR.

## Validation Checklist

Cross-check this list against `docs/proposals/durable-subscriber-verification.md` once its `Awakeable` section exists:

- [ ] Every phase above maps to one or more awakeable invariant IDs in `durable-subscriber-verification.md`.
- [ ] Phase 1 proves the Rust workflow handle is a thin projection of `DurableSubscriber::Passive`, not a second wait engine.
- [ ] Phase 2 proves `resolveAwakeable()` and `agent.resolvePermission()` append the same canonical completion-envelope shape.
- [ ] Phase 3 proves all public awakeable APIs use ACP SDK identifier types rather than raw `string`.
- [ ] Phase 3 examples preserve the agent-plane / infrastructure-plane split from [acp-canonical-identifiers.md](./acp-canonical-identifiers.md).
- [ ] Phase 4 proves replay reconstructs unresolved waits from the durable stream alone.
- [ ] Phase 4 proves step awakeables key by durable stream offset, not a Fireline counter or hash.
- [ ] No phase introduces a bespoke awakeable id, lineage table, or infrastructure-plane collection for imperative waits.
- [ ] Optional Phase 5 remains additive and does not change the correctness surface already verified in Phases 1-4.

## Architect Review Checklist

- [ ] The blocker is honored: implementation Phases 1-5 do not start before [durable-subscriber-execution.md Phase 7](./durable-subscriber-execution.md#phase-7-typescript-middleware-surface) is green on `main`.
- [ ] Phase 0 either extends an already-landed `durable-subscriber-verification.md` or is explicitly coupled to DurableSubscriber Phase 0 so the awakeable section has a real home.
- [ ] The plan does not duplicate DurableSubscriber trait, driver, or retry/dead-letter design; it only layers the imperative surface on top.
- [ ] Every awakeable key remains canonical: `PromptKey(SessionId, RequestId)`, `ToolKey(SessionId, ToolCallId)`, or `PromptStepKey(SessionId, RequestId, StreamOffset)`.
- [ ] Replay restoration depends only on the durable stream, not on process-local promise registries or synthetic ids.
- [ ] Approval-specific APIs remain thin specializations of generic awakeable resolution rather than a second mechanism.
- [ ] TypeScript examples remain visibly agent-plane only and do not surface infra-plane subscriber bookkeeping.
- [ ] Optional ergonomic sugar is kept strictly additive and can be skipped without blocking the core rollout.

## References

- [durable-promises.md](./durable-promises.md)
- [durable-subscriber.md](./durable-subscriber.md)
- [durable-subscriber-execution.md](./durable-subscriber-execution.md)
- [acp-canonical-identifiers.md](./acp-canonical-identifiers.md)
- [acp-canonical-identifiers-execution.md](./acp-canonical-identifiers-execution.md)
- [approval-gate-correctness.md](../reviews/approval-gate-correctness.md)
