# QA Review: Approval Gate After Phase 2 Canonical `RequestId`

Date: 2026-04-12

## Scope

This review validates the approval-gate behavior after Phase 2 switched the gate to canonical JSON-RPC `RequestId` keying:

- `074b14ea34ec618efeca2da3df9165234ca6d874` — `canonical-ids Phase 2: approval gate uses canonical JSON-RPC RequestId`
- `8d9d204e343358e42d1840d872e57d909cd500a3` — `canonical-ids Phase 2 fixup: align rebuild-race test with canonical RequestId`

CI-first rule followed: no local `cargo`. Evidence comes from GitHub Actions plus repo inspection with `gh` and `rg`.

## Overall Verdict

- Crash/resume: `PASS`
- Timeout: `PASS`
- Session isolation: `COVERAGE GAP IN CI`, no approval-gate regression signal from code inspection
- Rebuild-race: `PASS` after fixup; `FAIL` on base Phase 2 commit before the fixup
- Duplicate-approval / no fresh permission request after rebuild: `PASS`

Net: Phase 2 is functionally sound after `8d9d204`. The only remaining issue from a QA perspective is that GitHub Actions does not currently execute the pure `approval.rs` unit test that proves multi-session waiter isolation.

## CI Evidence

- Base Phase 2 run: `074b14e`
  - Run URL: https://github.com/gurdasnijor/fireline/actions/runs/24317332946
  - Result: `failure`
  - Relevant failure: `managed-agent-tests` failed in `tests/managed_agent_harness.rs`
  - Failing test: `harness_approval_resolution_during_rebuild_reuses_pending_request`

- Phase 2 fixup run: `8d9d204`
  - Run URL: https://github.com/gurdasnijor/fireline/actions/runs/24317388967
  - Workflow result: `failure` because unrelated `docker-host-images` failed
  - Relevant job: `managed-agent-tests` succeeded
  - Job URL: https://github.com/gurdasnijor/fireline/actions/runs/24317388967/job/70997372647

- Fresh validation rerun on descendant commit `739cbcb723c0feb073703ac5c37550b923b0aae6`
  - Run URL: https://github.com/gurdasnijor/fireline/actions/runs/24317820281
  - `739cbcb` contains both `074b14e` and `8d9d204`
  - Relevant job: `managed-agent-tests` succeeded
  - Job URL: https://github.com/gurdasnijor/fireline/actions/runs/24317820281/job/70998524562
  - Note: during evidence capture, the overall workflow was still waiting on unrelated `docker-host-images`; approval-gate evidence is in `managed-agent-tests`

## Invariant Review

### 1. Crash/Resume

Status: `PASS`

Evidence:

- Test: `harness_durable_suspend_resume_round_trip`
- Commit proven green: `8d9d204e343358e42d1840d872e57d909cd500a3`
- CI: `managed-agent-tests` in run `24317388967`
- Fresh descendant rerun: `managed-agent-tests` green in run `24317820281`

Why this is sufficient:

- This is the live managed-agent harness proof that a blocked approval survives runtime death, rebuilds from the durable log, and does not mint a new permission request after resume.

### 2. Timeout

Status: `PASS`

Evidence:

- Test: `harness_approval_gate_timeout_errors_cleanly`
- Commit proven green: `8d9d204e343358e42d1840d872e57d909cd500a3`
- CI: `managed-agent-tests` in run `24317388967`
- Fresh descendant rerun: `managed-agent-tests` green in run `24317820281`

Why this is sufficient:

- This proves the gate still returns the structured timeout error after canonical `RequestId` adoption and still avoids emitting a phantom `approval_resolved`.

### 3. Session Isolation

Status: `COVERAGE GAP IN CI`

Evidence:

- Canonical unit proof still exists: `concurrent_waiters_are_isolated_by_session_and_request_id`
- File: [approval.rs](/Users/gnijor/gurdasnijor/fireline/crates/fireline-harness/src/approval.rs:840)
- Current GitHub Actions surface does not run it
- `.github/workflows/managed-agent-suite.yml` only runs explicit integration test binaries and does not invoke crate unit tests

Assessment:

- I did not find a code regression in the approval gate itself.
- I also did not find a current GH Actions lane that re-proves this invariant after Phase 2.
- This is a CI coverage gap, not a confirmed functional regression.

Follow-up dispatch candidate:

- Add a CI job or step that runs:
  - `cargo test -p fireline-harness concurrent_waiters_are_isolated_by_session_and_request_id -- --exact`

### 4. Rebuild-Race

Status: `PASS` after fixup

Evidence:

- Base Phase 2 commit `074b14e` failed this invariant
  - Run: `24317332946`
  - Failing test: `harness_approval_resolution_during_rebuild_reuses_pending_request`
  - Failure: `deadline has elapsed` after external resolution during rebuild
- Fixup commit `8d9d204` repaired the invariant
  - Run: `24317388967`
  - Passing test: `harness_approval_resolution_during_rebuild_marks_session_approved`
- Fresh descendant rerun also has `managed-agent-tests` green
  - Run: `24317820281`

Why this is sufficient:

- This is exactly the Phase 2 regression that the fixup targeted. The failing base run and passing fixup run together show the problem and the repair.

### 5. Duplicate-Approval / No Fresh Permission Request After Rebuild

Status: `PASS`

Evidence:

- Test: `harness_approval_resolution_during_rebuild_marks_session_approved`
- Commit proven green: `8d9d204e343358e42d1840d872e57d909cd500a3`
- CI: `managed-agent-tests` in run `24317388967`
- Fresh descendant rerun: `managed-agent-tests` green in run `24317820281`

Why this is sufficient:

- That test asserts that once rebuild observes the canonical `approval_resolved`, follow-up prompts do not emit a fresh `permission_request`.
- It also asserts that the set of observed request ids remains the original singleton canonical id.

## Live Harness Confirmation

The CI evidence above is end-to-end managed-agent harness coverage, not just unit coverage:

- Workflow file: [managed-agent-suite.yml](/Users/gnijor/gurdasnijor/fireline/.github/workflows/managed-agent-suite.yml:51)
- It runs:
  - `--test managed_agent_harness`
  - `--test managed_agent_orchestration`
  - the rest of the managed-agent suite
- Those tests use the live harness helpers in [managed_agent_suite.rs](/Users/gnijor/gurdasnijor/fireline/tests/support/managed_agent_suite.rs:1993), which explicitly wait for the emitted `permission_request`, extract the canonical JSON-RPC `requestId`, and append `approval_resolved` back through the durable stream

This is the binding end-to-end proof that provision → prompt → approval block → external resolution → resume still works through the managed-agent harness after the canonical `RequestId` migration.

## Grep Review: Deleted Synthetic Identifier Paths

Approval-gate targeted grep:

- Command:
  - `rg -n "approval_request_id|prompt_identity|stable_prompt_identity|Sha256|sha256|approval_hash|fireline_trace_id" crates/fireline-harness/src/approval.rs tests/managed_agent_harness.rs tests/support/managed_agent_suite.rs`
- Result: zero hits

Broader code grep:

- Command:
  - `rg -n "approval_request_id|prompt_identity|stable_prompt_identity|Sha256|sha256|approval_hash|fireline_trace_id" crates tests packages -g '!target'`
- Result:
  - `crates/fireline-orchestration/src/child_session_edge.rs:7`
  - `crates/fireline-orchestration/src/child_session_edge.rs:79`

Assessment:

- No approval-gate stragglers remain for `approval_request_id`, `prompt_identity`, `stable_prompt_identity`, `approval_hash`, or `fireline_trace_id`.
- The remaining `Sha256` usage is in child-session edge hashing, outside the approval gate and outside the Phase 2 canonical `RequestId` migration.
- I do not flag that `child_session_edge.rs` use as an approval-gate regression.

## Regressions / Follow-Up Candidates

- No remaining approval-gate functional regression found after `8d9d204`
- CI coverage gap:
  - `concurrent_waiters_are_isolated_by_session_and_request_id` is not exercised by current GitHub Actions
  - Candidate follow-up: add a crate-unit-test lane for `fireline-harness`

## Conclusion

Phase 2 is correct after the fixup commit. The CI record shows the base Phase 2 landing broke rebuild-race, the fixup repaired it, and the fresh descendant rerun still has the managed-agent harness job green. The synthetic approval-id machinery is removed from the approval-gate path. The only remaining QA concern is coverage: GitHub Actions does not currently re-run the pure session-isolation unit proof.
