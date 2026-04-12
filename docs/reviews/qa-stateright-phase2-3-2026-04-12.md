# QA Review: Stateright Invariant Regression Post Phase 2+3

Date: 2026-04-12

## Scope

This review covers the Stateright portion of QA-4 only. The TLC/vacuity portion was already handled separately in `docs/reviews/qa-canonical-ids-invariants-2026-04-12.md`.

Inputs reviewed:
- `verification/stateright/`
- `docs/proposals/acp-canonical-identifiers-verification.md` §Stateright
- `docs/proposals/durable-subscriber-verification.md` §Stateright
- `docs/reviews/approval-gate-correctness.md`

Command run locally, using the isolated target dir required by the contention rules:

```sh
CARGO_TARGET_DIR=/tmp/fireline-w12 cargo test -p fireline-verification -- --nocapture --test-threads=1
```

## Result Summary

The existing Stateright crate is green on current `main`, but it does **not** yet implement the dedicated Phase 2 / Phase 3 models promised by the verification proposals.

Observed test run:

```text
running 6 tests
test tests::approval_protocol_model_checks_release_race_properties ... ok
test tests::cold_resume_model_checks_reprovision_properties ... ok
test tests::live_resume_model_checks_noop_and_single_winner_properties ... ok
test tests::registry_liveness_model_checks_unified_liveness_invariant ... ok
test tests::session_protocol_model_checks_core_session_invariants ... ok
test tests::stream_truth_model_checks_runtime_index_projection_invariant ... ok

test result: ok. 6 passed; 0 failed; 0 ignored
```

No flakiness was observed in this run. The whole Stateright crate completed in about `0.03s` after compilation.

## What Exists Today

`verification/stateright/` currently contains only:

```text
verification/stateright/Cargo.toml
verification/stateright/src/lib.rs
```

The active test matrix on current `main` is:
- `session_protocol_model_checks_core_session_invariants`
- `live_resume_model_checks_noop_and_single_winner_properties`
- `cold_resume_model_checks_reprovision_properties`
- `approval_protocol_model_checks_release_race_properties`
- `registry_liveness_model_checks_unified_liveness_invariant`
- `stream_truth_model_checks_runtime_index_projection_invariant`

Relevant current properties:
- `SessionReplayFromOffsetIsSuffix`
- `SessionDurableAcrossRuntimeDeath`
- `SessionScopedIdempotentAppend`
- `HarnessSuspendReleasedOnlyByMatchingApproval`
- `ApprovalDuplicateResolutionDoesNotDuplicateProgress`
- `ApprovalTerminalDecisionFollowsFirstMatchingResolution`

Those are real and green. But the proposal-defined canonical-id and durable-subscriber Stateright modules have not landed yet.

## Invariant Matrix

| Invariant | Expected Stateright test | Current Stateright test on `main` | Status | Evidence |
| --- | --- | --- | --- | --- |
| `DSV-01 CompletionKeyUnique` | `durable_subscriber_model_first_resolution_wins` or equivalent `FirstResolutionWins` property from `docs/proposals/durable-subscriber-verification.md` | No dedicated durable-subscriber test exists. The closest proxy is `approval_protocol_model_checks_release_race_properties`, which proves `ApprovalDuplicateResolutionDoesNotDuplicateProgress` and `ApprovalTerminalDecisionFollowsFirstMatchingResolution` for the abstract approval model. | `NOT YET IMPLEMENTED` | Proposal requires new `verification/stateright/src/durable_subscriber.rs`; that file does not exist. Current approval property names are in `verification/stateright/src/lib.rs:475-517`. |
| Session isolation under concurrent approvals | `canonical_ids_model_checks_concurrent_approval_scoping` from `docs/proposals/acp-canonical-identifiers-verification.md` | No dedicated canonical-ids test exists. Current approval Stateright model has no session dimension at all. It uses `ApprovalRequestId::{Expected, Noise}` only. | `NOT YET IMPLEMENTED` | Proposal names `ConcurrentApprovalsRemainSessionScoped` at `docs/proposals/acp-canonical-identifiers-verification.md:374-390`. Current approval model imports `ApprovalRequestId as RequestId` and operates on `ApprovalState` without `SessionId`; see `verification/stateright/src/lib.rs:16-24,431-518` and `crates/fireline-semantics/src/lib.rs:567-599`. |
| Replay idempotency | `durable_subscriber_model_rebuild_race_converges`, `canonical_ids_model_checks_replay_preserves_identifier_invariants`, or equivalent `ReplayIdempotent`/`ReplayPreservesCanonicalIdentifiers` property | `session_protocol_model_checks_core_session_invariants` passes and does cover a weaker replay/idempotency surface through `SessionReplayFromOffsetIsSuffix` and `SessionScopedIdempotentAppend`. But there is no dedicated canonical-id replay model and no durable-subscriber rebuild-race model. | `PASS` for the current session model; `NOT YET IMPLEMENTED` for the named Phase 2/3 invariant models | Pass evidence is the test run above plus `verification/stateright/src/lib.rs:267-303,687-691`. Missing-model evidence is the proposal text at `docs/proposals/acp-canonical-identifiers-verification.md:378-385` and `docs/proposals/durable-subscriber-verification.md:242-266`, both of which call for new tests that are absent. |
| Rebuild race convergence | `durable_subscriber_model_rebuild_race_converges` / `RebuildRaceConverges` | No dedicated Stateright rebuild-race property exists on current `main`. | `NOT YET IMPLEMENTED` | Expected by `docs/proposals/durable-subscriber-verification.md:242-247`. The current Stateright crate has no `durable_subscriber.rs`, and no property named `RebuildRaceConverges`. |
| Duplicate completion / first resolution wins | `durable_subscriber_model_first_resolution_wins` | `approval_protocol_model_checks_release_race_properties` | `PASS` as an approval-specific proxy only | Existing pass evidence is the test run plus `verification/stateright/src/lib.rs:485-517,705-711`. This is not yet the generalized completion-key model promised by the durable-subscriber proposal. |

## Assessment

### What the current green Stateright run really proves

1. Session replay and append dedupe are modeled and passing.
   `session_protocol_model_checks_core_session_invariants` proves:
   - replay from offset returns the suffix of the captured log
   - append history remains prefix-monotonic
   - commit tuples remain unique
   - crash does not erase previously durable log state

2. Approval resolution races are modeled and passing.
   `approval_protocol_model_checks_release_race_properties` proves:
   - blocked work is not released by a non-matching approval
   - duplicate approval resolutions do not produce duplicate progress
   - the first matching resolution determines terminal allow/deny

3. Resume, registry liveness, and runtime-index projection models are also green.
   Those were not the focus of this QA pass, but they do pass in the same run.

### What is still missing relative to the proposals

1. No canonical-ids Stateright model has landed.
   `docs/proposals/acp-canonical-identifiers-verification.md` explicitly calls for:
   - `verification/stateright/src/canonical_ids.rs`
   - `ConcurrentApprovalsRemainSessionScoped`
   - `ReplayPreservesCanonicalIdentifiers`
   - `PromptRequestRefUniquePerSession`

   None of those exist yet on current `main`.

2. No durable-subscriber Stateright model has landed.
   `docs/proposals/durable-subscriber-verification.md` explicitly calls for:
   - `verification/stateright/src/durable_subscriber.rs`
   - `FirstResolutionWins`
   - `RebuildRaceConverges`
   - `TimeoutAndResolutionAreAtomic`
   - `RetryBudgetTerminates`
   - `TraceContextSurvivesDelivery`

   None of those exist yet on current `main`.

3. The current approval Stateright model is still too abstract to prove session scoping.
   The proposal-level invariant is about `(SessionId, RequestId)` scoping. The current model does not carry `SessionId`; it only distinguishes `Expected` vs `Noise`. So its green result is useful, but it is not the same proof.

## Bottom Line

The Stateright crate on current `main` is healthy, and its existing models still pass after the canonical-ids Phase 2 / Phase 3 work landed.

But the specific post-Phase-2/3 invariants named in the verification proposals are only partially represented:
- replay/idempotent session behavior: covered by an existing proxy model and green
- approval first-resolution-wins behavior: covered by an existing approval-specific proxy model and green
- session-scoped concurrent approvals under canonical identifiers: **not yet implemented in Stateright**
- canonical replay identifier preservation: **not yet implemented in Stateright**
- durable-subscriber completion-key uniqueness / rebuild-race convergence: **not yet implemented in Stateright**

So the honest QA read is:
- current Stateright regressions: **none observed**
- proposal-required Stateright expansion for canonical ids + durable subscriber: **still pending**
