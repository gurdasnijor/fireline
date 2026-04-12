# ACP Canonical Identifiers Execution Plan

> Status: execution plan
> Date: 2026-04-12
> Scope: harness, session/state schema, approvals, peer lineage, Platform SDK, state packages

This document is the rollout plan for [acp-canonical-identifiers.md](./acp-canonical-identifiers.md). It is intentionally operational: each phase is small enough to land on `main`, has a hard verification gate, and is revertable on its own.

## Working Rules

1. Land directly on `main` as short-lived PRs. Do not build a long-lived refactor branch.
2. Do not advance to Phase `N+1` until Phase `N` is `cargo check`-green locally **and** the full test suite is green in CI. Per the CI-first contention rules (see [`docs/status/orchestration-status.md § Contention rules`](../status/orchestration-status.md), established 2026-04-12), dispatched codexes run `cargo check --workspace` only; GitHub Actions is the authoritative test environment. Gate on `gh run watch` / `gh run list --limit 1` before advancing.
3. Preserve behavior during migration. When a wire shape changes incompatibly, use versioned entity types or dual-read/dual-write adapters until cleanup.
4. Treat [acp-canonical-identifiers.md §14](./acp-canonical-identifiers.md#14-validation-checklist) and [approval-gate-correctness.md](../reviews/approval-gate-correctness.md) as the current invariant source. Phase 0 creates the missing dedicated verification doc so later phases can cite stable invariant IDs.
5. One phase per PR. Each PR must be revertable without partially reverting another phase.

## Compatibility Strategy

Use a mixed strategy:

- Type-only phases: additive only.
- Incompatible agent-plane row changes: write canonical rows under new entity names or versioned entity types first, then update readers, then delete old readers in Phase 8.
- Infrastructure-plane rows: keep their existing streams and ids; they are not ACP identifiers and are not part of the agent-plane migration.

This avoids rewriting append-only durable streams and keeps replay safe during the cutover.

## Adjusted Phase Order

This plan intentionally swaps the original “delete lineage structures” and “add W3C trace propagation” order:

- Phase 4 introduces canonical W3C trace-context propagation.
- Phase 5 deletes `ActiveTurnIndex` and `child_session_edge`.

Deleting the synthetic lineage structures first would break peer calls before a canonical replacement exists.

## Phase 0: Prerequisites

**A. Files touched**
- `docs/proposals/acp-canonical-identifiers-execution.md`
- `docs/proposals/acp-canonical-identifiers-verification.md` (new)
- `docs/proposals/acp-canonical-identifiers.md`

**B. Exact change summary**
- Create `acp-canonical-identifiers-verification.md` with stable invariant IDs copied from `acp-canonical-identifiers.md §14` and the approval review.
- Record the migration strategy decision: additive type layer first, then versioned entity types / dual-read for incompatible row changes, then cleanup.
- File an upstream ACP issue if Fireline still cannot access canonical `ToolCallId` at the tool-execution seam; add the issue link to both proposal docs.
- Confirm that the already-landed approval correctness tests still pass on `main` before any refactor work begins.

**C. Type-level change summary**
- No runtime type changes.
- Documentation only: establish the canonical types that later phases must use.

**D. Tests that must be added or updated**
- No new tests.
- Capture the current baseline by running the approval correctness tests below.

**E. Verification gate**
Run all four and record success in the PR:

```bash
cargo test --test managed_agent_harness harness_durable_suspend_resume_round_trip -- --exact
cargo test --test managed_agent_harness harness_approval_gate_timeout_errors_cleanly -- --exact
cargo test --test managed_agent_harness harness_approval_resolution_during_rebuild_reuses_pending_request -- --exact
cargo test -p fireline-harness concurrent_waiters_are_isolated_by_session_and_request_id -- --exact
```

**F. Estimated LOC of diff**
- 80-140

**G. Dependencies on prior phases**
- None

**H. Can be dispatched independently?**
- Yes. This phase is docs + baseline verification only.

**Rollback**
- Revert the docs commit. No runtime rollback needed.

## Phase 1: Shared Type Layer

**A. Files touched**
- `Cargo.toml`
- `crates/fireline-acp-ids/Cargo.toml` (new)
- `crates/fireline-acp-ids/src/lib.rs` (new)
- `crates/fireline-harness/Cargo.toml`
- `crates/fireline-session/Cargo.toml`
- `crates/fireline-tools/Cargo.toml`
- `crates/fireline-orchestration/Cargo.toml`
- `packages/client/src/types.ts`
- `packages/client/src/index.ts`
- `packages/state/src/acp-types.ts` (new)
- `packages/state/src/index.ts`

**B. Exact change summary**
- Add a tiny Rust crate `fireline-acp-ids` that re-exports `sacp::schema::{SessionId, RequestId, ToolCallId}` and defines `PromptRequestRef` and `ToolInvocationRef`.
- Add a TypeScript ACP-id shim module that re-exports the ACP SDK branded identifier types and parallel `PromptRequestRef` / `ToolInvocationRef` helper types.
- Export those helpers from `@fireline/client` and `@fireline/state`.
- Do not change behavior or wire formats yet. This phase is only additive plumbing.

**C. Type-level change summary**
- Rust: introduce `fireline_acp_ids::{SessionId, RequestId, ToolCallId, PromptRequestRef, ToolInvocationRef}`.
- TypeScript: export ACP-branded `SessionId`, `RequestId`, `ToolCallId`, `PromptRequestRef`, and `ToolInvocationRef`.

**D. Tests that must be added or updated**
- Add one Rust unit test in `crates/fireline-acp-ids/src/lib.rs` for serde round-trip of a `PromptRequestRef`.
- Add one TS compile-only test or fixture that imports the new types from both `@fireline/client` and `@fireline/state`.

**E. Verification gate**
```bash
cargo check --workspace
pnpm --filter @fireline/client build
pnpm --filter @fireline/state build
```

**F. Estimated LOC of diff**
- 120-220

**G. Dependencies on prior phases**
- Phase 0

**H. Can be dispatched independently?**
- Yes, after Phase 0. It is additive and should not overlap behavior-changing work.

**Rollback**
- Revert the new crate/module exports. No stream compatibility concerns.

## Phase 2: Approval Gate First

**A. Files touched**
- `crates/fireline-harness/src/approval.rs`
- `tests/managed_agent_harness.rs`
- `tests/support/managed_agent_suite.rs`
- `packages/client/src/events.ts`
- `packages/client/src/agent.ts`

**B. Exact change summary**
- Delete `approval_request_id()`, `stable_prompt_identity()`, and the SHA256-based request-id derivation path from `approval.rs`.
- Parse the actual JSON-RPC id from the intercepted `request_permission` request and use it as the canonical `RequestId`.
- Key `PendingApproval`, `emit_permission_request`, `wait_for_approval`, and `rebuild_from_log` on `(SessionId, RequestId)`.
- During the migration window, if permission rows still carry both `requestId` and `jsonrpcId`, write the same canonical ACP id to both fields so existing TS readers do not break before Phase 6.
- Update `appendApprovalResolved()` and `FirelineAgent.resolvePermission()` signatures to use the new ACP id aliases, not plain strings.

**C. Type-level change summary**
- `PendingApproval.request_id: String -> fireline_acp_ids::RequestId`
- `emit_permission_request`, `wait_for_approval`, `approval_timeout_error`: plain string ids -> typed `SessionId` / `RequestId`
- TS `appendApprovalResolved()` and `resolvePermission()` accept ACP-branded `SessionId` / `RequestId`

**D. Tests that must be added or updated**
- Update the existing approval unit tests in `approval.rs` to assert canonical ids, not hashed ids.
- Keep these green:
  - `concurrent_waiters_are_isolated_by_session_and_request_id`
  - `harness_approval_gate_timeout_errors_cleanly`
  - `harness_durable_suspend_resume_round_trip`
  - `harness_approval_resolution_during_rebuild_reuses_pending_request`
- Add one new unit test in `approval.rs` that proves the emitted permission row preserves the exact incoming JSON-RPC id, including string ids that are not UUIDs.

**E. Verification gate**
```bash
cargo test -p fireline-harness concurrent_waiters_are_isolated_by_session_and_request_id -- --exact
cargo test --test managed_agent_harness harness_approval_gate_timeout_errors_cleanly -- --exact
cargo test --test managed_agent_harness harness_durable_suspend_resume_round_trip -- --exact
cargo test --test managed_agent_harness harness_approval_resolution_during_rebuild_reuses_pending_request -- --exact
rg -n "approval_request_id|approval_hash|Sha256|sha256|stable_prompt_identity|fireline_trace_id" crates/fireline-harness/src/approval.rs
```

The final `rg` must return zero matches.

**F. Estimated LOC of diff**
- 160-260

**G. Dependencies on prior phases**
- Phase 1

**H. Can be dispatched independently?**
- Yes, after Phase 1. This is the smallest behavior-changing slice and should land before any projector/schema refactor.

**Rollback**
- Revert the approval gate commit only.
- Because this phase does not yet delete old state entities, rollback is safe as long as the reverted code still accepts the current permission rows.

## Phase 3: StateProjector Canonical Rekeying

**A. Files touched**
- `crates/fireline-harness/src/state_projector.rs`
- `crates/fireline-harness/src/routes_acp.rs`
- `crates/fireline-harness/src/trace.rs`
- `crates/fireline-session/src/lib.rs`
- `tests/managed_agent_harness.rs`
- `tests/managed_agent_sandbox.rs`
- `tests/minimal_vertical_slice.rs`
- `tests/state_fixture_snapshot.rs`

**B. Exact change summary**
- Replace `PromptTurnRow` with `PromptRequestRow` in the Rust projector.
- Stop minting `logical_connection_id` in `routes_acp.rs`.
- Delete `next_prompt_turn_id()`, `turn_counter`, `prompt_request_to_turn`, `session_active_turn`, `chunk_seq`, and `InheritedLineage`.
- Re-key prompt lifecycle rows by the canonical `(session_id, request_id)` pair. Use a derived storage key such as `{session_id}:{request_id}` only as a storage convenience.
- Delete synthetic `chunk_id` and `seq`; ordering comes from durable-stream offsets.
- Delete `trace_id` and `parent_prompt_turn_id` from `SessionRecord`.
- During the migration window, dual-write canonical rows under new entity names:
  - `prompt_request`
  - `chunk_v2`
  - `session_v2`
- Keep old `prompt_turn`, `chunk`, and `session` readers alive until Phase 8; do not delete them in this phase.

**C. Type-level change summary**
- `PromptTurnRow -> PromptRequestRow`
- `session_id`, `request_id`: plain strings -> ACP types
- `SessionRecord` drops `logical_connection_id`, `trace_id`, `parent_prompt_turn_id`
- `ChunkRow` becomes keyed by `SessionId + RequestId (+ ToolCallId when present)` rather than `prompt_turn_id`

**D. Tests that must be added or updated**
- Add a focused projector unit test proving that two prompts in one session produce two distinct canonical `prompt_request` rows keyed by request id.
- Update `tests/managed_agent_harness.rs` and `tests/managed_agent_sandbox.rs` to assert `prompt_request` / canonical request-id behavior.
- Update `tests/state_fixture_snapshot.rs` to include canonical entity types in the fixture output.

**E. Verification gate**
```bash
cargo test --workspace
rg -n "next_prompt_turn_id|turn_counter|prompt_request_to_turn|chunk_seq|InheritedLineage|logical_connection_id" crates/fireline-harness/src/state_projector.rs crates/fireline-harness/src/routes_acp.rs crates/fireline-harness/src/trace.rs
rg -n "trace_id|parent_prompt_turn_id" crates/fireline-harness/src/state_projector.rs crates/fireline-session/src/lib.rs
```

Both `rg` commands must return zero matches in source files.

**F. Estimated LOC of diff**
- 350-600

**G. Dependencies on prior phases**
- Phase 2

**H. Can be dispatched independently?**
- No. It is serial after Phase 2 because approval rows and projector rows must agree on canonical `RequestId`.

**Rollback**
- Revert the projector PR only.
- Because this phase dual-writes new entity types rather than mutating old rows in place, rollback is safe: old readers still exist.

## Phase 4: W3C Trace Context Propagation

**A. Files touched**
- `Cargo.toml`
- `crates/fireline-tools/Cargo.toml`
- `crates/fireline-harness/Cargo.toml`
- `crates/fireline-tools/src/peer/transport.rs`
- `crates/fireline-tools/src/peer/mcp_server.rs`
- `crates/fireline-tools/src/peer/lookup.rs`
- `crates/fireline-harness/src/approval.rs`
- `crates/fireline-harness/src/routes_acp.rs`
- `tests/mesh_baseline.rs` or `tests/peer_trace_context.rs` (new)

**B. Exact change summary**
- Add `opentelemetry` to the workspace and thread a minimal trace-context helper through peer transport.
- Replace `_meta.fireline.traceId` and `_meta.fireline.parentPromptTurnId` propagation with root-level ACP `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage`.
- On outbound peer calls, inject W3C trace context into the ACP request.
- On inbound peer calls, extract W3C trace context before opening the downstream span.
- Emit spans for:
  - `session/new`
  - `session/prompt`
  - tool call
  - peer call outbound/inbound
  - approval emit / approval resolve

**C. Type-level change summary**
- `ParentLineage { trace_id, parent_prompt_turn_id } -> TraceContextCarrier { traceparent, tracestate?, baggage? }`
- `PeerCallResult.child_session_id: String -> SessionId`

**D. Tests that must be added or updated**
- Add a new peer integration test that provisions two runtimes, performs a `prompt_peer` call, and asserts that the child request carries the same `trace-id` via W3C trace context rather than Fireline-specific `_meta` keys.
- Update any existing peer transport tests that inspect `_meta.fireline.*`.

**E. Verification gate**
```bash
cargo check --workspace
cargo test --test mesh_baseline peer_trace_context_propagates_across_prompt_peer -- --exact
rg -n "\"traceId\"|\"parentPromptTurnId\"|_meta\\.fireline" crates/fireline-tools/src/peer crates/fireline-harness/src
```

The `rg` command must return zero source matches.

**F. Estimated LOC of diff**
- 220-380

**G. Dependencies on prior phases**
- Phase 3

**H. Can be dispatched independently?**
- No. The peer transport, harness, and integration tests must move together.

**Rollback**
- Revert the trace-context PR only.
- This phase is isolated because Phase 5 does not land until this gate is green.

## Phase 5: Delete `ActiveTurnIndex` and `child_session_edge`

**A. Files touched**
- `crates/fireline-session/src/active_turn_index.rs` (delete)
- `crates/fireline-session/src/lib.rs`
- `crates/fireline-orchestration/src/child_session_edge.rs` (delete)
- `crates/fireline-orchestration/src/lib.rs`
- `crates/fireline-host/src/bootstrap.rs`
- `crates/fireline-harness/src/host_topology.rs`
- `crates/fireline-tools/src/peer/lookup.rs`
- `crates/fireline-tools/src/peer/mcp_server.rs`
- `tests/mesh_baseline.rs`
- `tests/control_plane_docker.rs`

**B. Exact change summary**
- Delete `ActiveTurnIndex`, its exports, and all bootstrap wiring that keeps it alive.
- Delete `child_session_edge.rs`, `ChildSessionEdgeWriter`, and all edge emission from peer calls.
- Remove `ActiveTurnLookup` and `ChildSessionEdgeSink` from peer interfaces.
- Update the peer call flow to depend only on the canonical `SessionId` and W3C trace context introduced in Phase 4.
- Rewrite mesh and docker integration tests to assert trace propagation and session creation, not bespoke edge rows.

**C. Type-level change summary**
- Delete `ActiveTurnRecord`
- Delete `ChildSessionEdgeInput`
- Remove `prompt_turn_id`, `trace_id`, and `parent_prompt_turn_id` from peer interfaces

**D. Tests that must be added or updated**
- Delete unit tests that belong to `active_turn_index.rs` and `child_session_edge.rs`.
- Update:
  - `tests/mesh_baseline.rs`
  - `tests/control_plane_docker.rs`
- Add a regression test that a peer call still provisions a child session and that the trace link is visible without any `child_session_edge` row.

**E. Verification gate**
```bash
cargo check --workspace
rg -n "active_turn_index|ActiveTurnIndex|child_session_edge|ChildSessionEdge|prompt_turn_id|parent_prompt_turn_id" crates/fireline-session crates/fireline-tools crates/fireline-host crates/fireline-orchestration tests/mesh_baseline.rs tests/control_plane_docker.rs
```

The `rg` command must return zero source matches.

**F. Estimated LOC of diff**
- 220-420

**G. Dependencies on prior phases**
- Phase 4

**H. Can be dispatched independently?**
- No. This is a pure deletion phase, but only after trace propagation exists.

**Rollback**
- Revert the deletion PR only.
- Because Phase 4 already introduced the canonical trace path, rollback is straightforward if a peer-flow regression appears.

## Phase 6: TypeScript Schema Migration

**A. Files touched**
- `packages/state/src/schema.ts`
- `packages/state/src/collection.ts`
- `packages/state/src/index.ts`
- `packages/state/src/collections/session-turns.ts`
- `packages/state/src/collections/turn-chunks.ts`
- `packages/state/src/collections/active-turns.ts`
- `packages/state/src/collections/queued-turns.ts`
- `packages/state/src/collections/connection-turns.ts`
- `packages/state/test/schema.test.ts`
- `packages/state/test/collections.test.ts`
- `packages/state/test/rust-fixture.test.ts`
- `packages/client/src/db.ts`
- `packages/client/src/index.ts`

**B. Exact change summary**
- Add canonical TS row types:
  - `PromptRequestRow`
  - `PermissionRow` keyed by canonical `RequestId`
  - `SessionRow` with agent-plane fields only
  - `ChunkRow` keyed by `(sessionId, requestId, toolCallId?)`
- Replace `promptTurns` with `promptRequests` in the public DB collections.
- Remove `childSessionEdges` from the public state schema.
- Add renamed query builders:
  - `createSessionPromptRequestsCollection`
  - `createRequestChunksCollection`
- Keep deprecated alias exports for one phase:
  - `PromptTurnRow = PromptRequestRow`
  - `createSessionTurnsCollection = createSessionPromptRequestsCollection`
  - `createTurnChunksCollection = createRequestChunksCollection`
- Make the schema dual-read old and new row shapes during the migration window.

**C. Type-level change summary**
- Public TS row types use ACP-branded `SessionId`, `RequestId`, `ToolCallId`
- `jsonrpcId`, `promptTurnId`, `logicalConnectionId`, `traceId`, `parentPromptTurnId` disappear from public agent-plane row types
- `childSessionEdges` disappears from `FirelineCollections`

**D. Tests that must be added or updated**
- Update `schema.test.ts` to validate `prompt_request` and canonical permission/session/chunk rows.
- Update `collections.test.ts` to target `promptRequests` and request-keyed chunks.
- Update `rust-fixture.test.ts` to accept the new canonical fixture output and to reject `child_session_edge`.

**E. Verification gate**
```bash
pnpm --filter @fireline/state build
pnpm --filter @fireline/state test
pnpm --filter @fireline/client build
```

If `@fireline/client` tests exercise the migrated DB surface, also run:

```bash
pnpm --filter @fireline/client test
```

**F. Estimated LOC of diff**
- 300-520

**G. Dependencies on prior phases**
- Phase 5

**H. Can be dispatched independently?**
- Yes, once Phase 5 lands. This is primarily a TS/package migration.

**Rollback**
- Revert the TS migration PR.
- Because deprecated aliases remain for one phase, rollback is low-risk.

## Phase 7: Plane Separation Enforcement

**A. Files touched**
- `crates/fireline-session/src/lib.rs`
- `crates/fireline-session/src/session_index.rs`
- `crates/fireline-host/src/bootstrap.rs`
- `packages/client/src/admin.ts`
- `packages/client/src/db.ts`
- `packages/state/src/schema.ts`
- `packages/state/src/collection.ts`
- `packages/client/test/db-plane-separation.test.ts` (new)

**B. Exact change summary**
- Strip infrastructure fields from agent-plane session rows:
  - remove `host_key`
  - remove `host_id`
  - remove `node_id`
- Stop having `SessionIndex` materialize `runtime_spec` rows or join sessions back to host specs.
- Keep infrastructure state in the infrastructure plane only:
  - `hosts:tenant-{id}`
  - `sandboxes:tenant-{id}`
- Make the explicit infra read surfaces live under admin APIs:
  - `admin.listHosts()`
  - `admin.listSandboxes()`
- Ensure `fireline.db()` exposes only agent-plane collections and no infrastructure rows or fields.

**C. Type-level change summary**
- `SessionRecord` becomes agent-plane only.
- `SandboxAdmin` gains explicit infrastructure read methods.
- `FirelineCollections` contains zero infrastructure-bearing fields.

**D. Tests that must be added or updated**
- Add `packages/client/test/db-plane-separation.test.ts` that preloads `fireline.db()` and asserts no collection row exposes `hostKey`, `runtimeKey`, `hostId`, `runtimeId`, or `nodeId`.
- Update `crates/fireline-session/src/session_index.rs` unit tests to stop joining host specs through session rows.

**E. Verification gate**
```bash
cargo check --workspace
pnpm --filter @fireline/client build
pnpm --filter @fireline/client exec vitest run test/db-plane-separation.test.ts
```

The new test must prove that `fireline.db()` returns zero infrastructure fields across all public collections.

**F. Estimated LOC of diff**
- 180-320

**G. Dependencies on prior phases**
- Phase 6

**H. Can be dispatched independently?**
- No. This phase touches Rust agent-plane rows and TS public APIs together.

**Rollback**
- Revert the plane-separation PR.
- Because Phase 6 still kept migration aliases, rollback is safe if downstream apps break.

## Phase 8: Cleanup and Delete Migration Scaffolding

**A. Files touched**
- All temporary dual-read / dual-write branches introduced in Phases 2-7
- `packages/state/src/index.ts`
- `packages/client/src/index.ts`
- `packages/state/test/fixtures/*`
- `tests/state_fixture_snapshot.rs`
- `docs/proposals/acp-canonical-identifiers.md`
- `docs/proposals/acp-canonical-identifiers-verification.md`

**B. Exact change summary**
- Delete deprecated readers/writers for:
  - `prompt_turn`
  - old `permission` rows with duplicated `jsonrpcId`
  - old `session` rows with synthetic lineage / infra baggage
  - old `chunk` rows
- Remove any versioned entity names if introduced only for migration.
- Remove deprecated TS alias exports.
- Delete any remaining synthetic transport/state types such as `ConnectionRow` if they survived earlier phases only for compatibility.
- Mark the canonical-identifiers proposal as implemented and reduce the verification doc to the permanent regression suite.

**C. Type-level change summary**
- The public steady state exposes only canonical ACP id types on the agent plane.
- No synthetic identity fields or compatibility aliases remain.

**D. Tests that must be added or updated**
- Finalize fixture snapshots to canonical rows only.
- Update any lingering tests that still reference old entity names or aliases.

**E. Verification gate**
```bash
cargo test --workspace
pnpm --filter @fireline/state test
pnpm --filter @fireline/client test
rg -n "prompt_turn_id|trace_id|parent_prompt_turn_id|logical_connection_id|chunk_id|chunk_seq|child_session_edge|approval_request_id|approval_hash|jsonrpcId" crates packages tests
```

The `rg` audit must return zero matches in source and test code, excluding this proposal doc and other historical docs.

Additionally, every checklist item in [acp-canonical-identifiers.md §14](./acp-canonical-identifiers.md#14-validation-checklist) must be marked green in `acp-canonical-identifiers-verification.md`.

**F. Estimated LOC of diff**
- 200-320

**G. Dependencies on prior phases**
- Phase 7

**H. Can be dispatched independently?**
- No. This is the final serial cleanup pass.

**Rollback**
- Revert the cleanup PR and restore the previous dual-read release.
- Do not partially reintroduce deleted compatibility code; revert the entire phase if CI or replay behavior regresses.

## Dispatch Model

Each phase can be handed to an agent with exactly three inputs:

1. this execution doc
2. the canonical proposal
3. the approval correctness review

The dispatch contract is:

- do only the listed files
- do not fold later phases forward
- run the listed gate exactly
- stop if the gate fails and report the first invariant that broke

## Release / Rollback Guidance

- Merge phases one at a time to `main`.
- Let CI run after every phase.
- If a subtle replay bug appears after merge, revert the entire phase PR rather than cherry-picking file-level pieces.
- Do not cut a long-lived “canonical-identifiers” branch. The safety comes from small revertable steps, not from batching the refactor.
