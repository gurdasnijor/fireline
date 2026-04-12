# ACP Canonical Identifiers Verification

> Status: design
> Date: 2026-04-12
> Scope: verification additions for `docs/proposals/acp-canonical-identifiers.md`

This document specifies the verification work needed to make regressions in the ACP Canonical Identifiers refactor mechanically detectable.

It is intentionally redundant across layers:

- TLA+ proves the protocol shape and the identifier invariants.
- Stateright checks race and replay properties against a refinement of the Rust execution model.
- Mechanical audits fail the build when code drifts back to synthetic ids or plane leakage.
- Migration fixtures prove the dual-read window and the cutover behavior.
- End-to-end tests prove the happy path, peer lineage, and resume-after-death story against the live stack.

## Part 1: TLA+ Extensions

Target files: `verification/spec/managed_agents.tla`, `verification/spec/ManagedAgents.cfg`, new `verification/spec/ManagedAgentsCanonicalIds.cfg`.

### 1.1 Required model edits

Apply these structural edits before adding new invariants:

1. Rename the event kind `"prompt_turn_started"` to `"prompt_request_started"` everywhere in `managed_agents.tla`.
2. Remove synthetic agent-plane identifiers from the model shape:
   - delete `LogicalEventIds`
   - delete the `logicalId` field from `sessionLog` entries and `visibleEffects`
   - do not introduce any variable or record field named `prompt_turn_id`, `trace_id`, `parent_prompt_turn_id`, or `logical_connection_id`
3. Extend the identifier carrier sets:

```tla
CONSTANTS
  Sessions,
  RuntimeKeys,
  RuntimeIds,
  NodeIds,
  ProviderInstanceIds,
  SandboxIds,
  RequestIds,
  ToolCallIds,
  ToolNames,
  Sources,
  MountPaths,
  ProducerIds,
  ProducerEpochs,
  ProducerSeqs,
  Callers,
  TraceParents,
  TraceStates,
  Baggages
```

4. Extend `runtimeIndex` rows so infrastructure-plane rows can carry the ids we want to prove are disjoint from the agent plane:

```tla
InitialRuntimeIndex ==
  [ rk \in RuntimeKeys |->
      [ status |-> "stopped",
        runtimeId |-> DefaultRuntimeId,
        nodeId |-> DefaultNodeId,
        providerInstanceId |-> DefaultProviderInstanceId,
        specPresent |-> FALSE,
        boundSessions |-> {}
      ]
  ]
```

5. Add a separate `_meta`-passthrough variable for cross-session lineage. This must be a real spec variable, not a derived operator:

```tla
NoToolCall == "no_tool_call"
NoTraceparent == "no_traceparent"
NoTracestate == "no_tracestate"
NoBaggage == "no_baggage"

InitialTraceContext ==
  [ s \in Sessions |->
      [ req \in RequestIds |->
          [ traceparent |-> NoTraceparent,
            tracestate |-> NoTracestate,
            baggage |-> NoBaggage
          ]
      ]
  ]
```

And then:

```tla
VARIABLES
  ...,
  traceContext,
  ...

Vars ==
  << ..., traceContext, ... >>

Init ==
  /\ ...
  /\ traceContext = InitialTraceContext
```

6. Change event payloads so the session log carries canonical identifiers directly:
   - `"session_created"` events carry `session_id`
   - `"prompt_request_started"` events carry `session_id`, `request_id`
   - `"permission_requested"` and `"approval_resolved"` events carry `session_id`, `request_id`, `tool_call_id` with `NoToolCall` when absent
   - `"chunk_appended"` events carry `session_id`, `request_id`, `tool_call_id` with `NoToolCall` when absent
7. Update `HarnessEveryEffectLogged` to match on commit tuple plus event kind and canonical ids, not on a synthetic `logicalId`.

### 1.2 Helper definitions

Add these helpers to `managed_agents.tla`:

```tla
DefaultNodeId == CHOOSE nid \in NodeIds : TRUE
DefaultProviderInstanceId == CHOOSE pid \in ProviderInstanceIds : TRUE

AgentIdentifierUniverse == Sessions \cup RequestIds \cup ToolCallIds
InfrastructureIdentifierUniverse ==
  RuntimeKeys \cup RuntimeIds \cup NodeIds \cup ProviderInstanceIds

SyntheticIdFields ==
  {"prompt_turn_id", "trace_id", "parent_prompt_turn_id",
   "logical_connection_id", "chunk_id", "chunk_seq", "seq", "edge_id"}

InfrastructureIdFields ==
  {"host_key", "runtime_id", "node_id", "provider_instance_id"}

AgentIdFields == {"session_id", "request_id", "tool_call_id"}

CrossSessionLineageFields ==
  {"trace_id", "parent_prompt_turn_id", "parent_session_id",
   "child_session_id", "logical_connection_id", "edge_id"}

AgentIdentifiers(row) ==
  (IF "session_id" \in DOMAIN row THEN {row.session_id} ELSE {})
  \cup (IF "request_id" \in DOMAIN row THEN {row.request_id} ELSE {})
  \cup (IF "tool_call_id" \in DOMAIN row /\ row.tool_call_id # NoToolCall
        THEN {row.tool_call_id}
        ELSE {})

InfrastructureIdentifiers(row) ==
  (IF "host_key" \in DOMAIN row THEN {row.host_key} ELSE {})
  \cup (IF "runtime_id" \in DOMAIN row THEN {row.runtime_id} ELSE {})
  \cup (IF "node_id" \in DOMAIN row THEN {row.node_id} ELSE {})
  \cup (IF "provider_instance_id" \in DOMAIN row
        THEN {row.provider_instance_id}
        ELSE {})

SessionRows ==
  { [session_id |-> s] :
      s \in Sessions /\
      \E i \in 1..Len(sessionLog[s]) :
        sessionLog[s][i].kind = "session_created" }

PromptRequestRows ==
  { [session_id |-> s,
     request_id |-> sessionLog[s][i].request_id] :
      s \in Sessions /\
      i \in 1..Len(sessionLog[s]) /\
      sessionLog[s][i].kind = "prompt_request_started" }

PermissionRows ==
  { [session_id |-> s,
     request_id |-> sessionLog[s][i].request_id,
     tool_call_id |-> sessionLog[s][i].tool_call_id] :
      s \in Sessions /\
      i \in 1..Len(sessionLog[s]) /\
      sessionLog[s][i].kind \in {"permission_requested", "approval_resolved"} }

ChunkRows ==
  { [session_id |-> s,
     request_id |-> sessionLog[s][i].request_id,
     tool_call_id |-> sessionLog[s][i].tool_call_id] :
      s \in Sessions /\
      i \in 1..Len(sessionLog[s]) /\
      sessionLog[s][i].kind = "chunk_appended" }

PendingRequestRows ==
  { [session_id |-> s,
     request_id |-> blockedRequests[s]] :
      s \in Sessions /\ blockedRequests[s] # NoRequest }

AgentRows ==
  SessionRows \cup PromptRequestRows \cup PermissionRows \cup ChunkRows \cup PendingRequestRows

HostRows ==
  { [host_key |-> rk,
     runtime_id |-> runtimeIndex[rk].runtimeId,
     node_id |-> runtimeIndex[rk].nodeId,
     provider_instance_id |-> runtimeIndex[rk].providerInstanceId] :
      rk \in RuntimeKeys /\ runtimeIndex[rk].specPresent }

InfrastructureRows == HostRows

PermissionRequestIds ==
  { sessionLog[s][i].request_id :
      s \in Sessions /\
      i \in 1..Len(sessionLog[s]) /\
      sessionLog[s][i].kind = "permission_requested" }

ResolvedApprovalIds ==
  { sessionLog[s][i].request_id :
      s \in Sessions /\
      i \in 1..Len(sessionLog[s]) /\
      sessionLog[s][i].kind = "approval_resolved" }

ChunkOrdinal(s, req, i) ==
  Cardinality(
    { j \in 1..i :
        /\ sessionLog[s][j].kind = "chunk_appended"
        /\ sessionLog[s][j].request_id = req
    }
  )

CanonicalIdsSmallModel ==
  /\ Cardinality(Sessions) <= 3
  /\ Cardinality(RequestIds) <= 5
  /\ Cardinality(ToolCallIds) <= 3
  /\ \A s \in Sessions : Len(sessionLog[s]) <= 4
  /\ \A s \in Sessions : lastReplay[s].offset <= 4
```

`ManagedAgentsCanonicalIds.cfg` should bind the new constants and add:

```tla
CONSTRAINT CanonicalIdsSmallModel
```

### 1.3 New invariants

#### AgentLayerIdentifiersAreCanonical

Formula:

```tla
AgentLayerIdentifiersAreCanonical ==
  /\ \A row \in AgentRows :
       /\ DOMAIN row \cap SyntheticIdFields = {}
       /\ AgentIdentifiers(row) \subseteq AgentIdentifierUniverse
  /\ \A row \in PermissionRows \cup ChunkRows :
       row.tool_call_id = NoToolCall \/ row.tool_call_id \in ToolCallIds
```

What it forbids: any agent-layer row carrying `prompt_turn_id`, `trace_id`, `parent_prompt_turn_id`, `logical_connection_id`, `chunk_id`, `chunk_seq`, `seq`, or `edge_id`; any agent-layer identifier outside `Sessions`, `RequestIds`, or `ToolCallIds`; any approval or chunk row carrying a synthesized tool-call id.

Proof strategy: exhaustive TLC run with `ManagedAgentsCanonicalIds.cfg`.

Counterexample: a `chunk_appended` row like `{session_id = "session_a", request_id = "request_a", chunk_id = "uuid-1"}` or a `permission_requested` row with `request_id = "sha256(...)"`.

#### InfrastructureAndAgentPlanesDisjoint

Formula:

```tla
InfrastructureAndAgentPlanesDisjoint ==
  /\ \A row \in AgentRows :
       /\ DOMAIN row \cap InfrastructureIdFields = {}
       /\ AgentIdentifiers(row) \cap InfrastructureIdentifierUniverse = {}
  /\ \A row \in InfrastructureRows :
       /\ DOMAIN row \cap AgentIdFields = {}
       /\ InfrastructureIdentifiers(row) \cap AgentIdentifierUniverse = {}
```

What it forbids: `host_key`, `runtime_id`, `node_id`, or `provider_instance_id` leaking onto agent-plane rows, and `session_id`, `request_id`, or `tool_call_id` leaking onto infrastructure rows.

Proof strategy: TLC exhaustive check under the same small config.

Counterexample: `SessionRows` contains `{session_id = "session_a", host_key = "runtime_key_a"}` or `HostRows` contains `{host_key = "runtime_key_a", session_id = "session_a"}`.

#### CrossSessionLineageIsOutOfBand

Formula:

```tla
CrossSessionLineageIsOutOfBand ==
  /\ \A row \in AgentRows :
       DOMAIN row \cap CrossSessionLineageFields = {}
  /\ \A s \in Sessions :
       \A req \in RequestIds :
         traceContext[s][req] \in
           [ traceparent : TraceParents \cup {NoTraceparent},
             tracestate : TraceStates \cup {NoTracestate},
             baggage : Baggages \cup {NoBaggage} ]
```

What it forbids: row-level lineage fields such as `trace_id`, `parent_prompt_turn_id`, `parent_session_id`, `child_session_id`, `logical_connection_id`, or `edge_id`, and any cross-session causal link outside `traceContext`.

Proof strategy: TLC exhaustive check under the small config.

Counterexample: a peer-call row includes `parent_session_id = "session_a"` or the model introduces a `child_session_edge`-like record into `AgentRows`.

#### ChunkOrderingFromStreamOffset

Formula:

```tla
ChunkOrderingFromStreamOffset ==
  /\ \A row \in ChunkRows : DOMAIN row \cap {"chunk_id", "chunk_seq", "seq"} = {}
  /\ \A s \in Sessions :
       \A req \in RequestIds :
         \A i, j \in 1..Len(sessionLog[s]) :
           /\ i < j
           /\ sessionLog[s][i].kind = "chunk_appended"
           /\ sessionLog[s][j].kind = "chunk_appended"
           /\ sessionLog[s][i].request_id = req
           /\ sessionLog[s][j].request_id = req
           => ChunkOrdinal(s, req, i) < ChunkOrdinal(s, req, j)
```

What it forbids: explicit chunk sequence fields in the row shape and any model where chunk ordering depends on stored counters rather than session-log position.

Proof strategy: TLC on bounded logs with `Len(sessionLog[s]) <= 4`.

Counterexample: two chunks for the same request appear at offsets `i < j`, but the materialized rows carry explicit `seq`, or the row shape still includes `chunk_id`.

#### ApprovalKeyedByCanonicalRequestId

Formula:

```tla
ApprovalKeyedByCanonicalRequestId ==
  /\ PermissionRequestIds \subseteq RequestIds
  /\ ResolvedApprovalIds \subseteq PermissionRequestIds
  /\ \A req \in RequestIds :
       pendingApprovals[req].state # "none" =>
         /\ req \in PermissionRequestIds
         /\ pendingApprovals[req].sessionId \in Sessions
```

What it forbids: hashed or counter-minted approval request ids, `approval_resolved` events that do not correspond to a real `permission_requested` request id, and approvals attached to non-session or synthesized ids.

Proof strategy: TLC exhaustive check under the same small config. If the model later splits prompt-request ids and permission-request ids into separate sets, keep the same invariant shape and swap in the narrower carrier set.

Counterexample: `RequestApproval` mints `req = "9a6b4..."` outside `RequestIds`, or `ResolveApproval` writes a `request_id` that was never emitted by `request_permission`.

### 1.4 Existing invariant updates

Update these existing checks at the same time:

- `HarnessEveryEffectLogged`: match on `(producerId, epoch, seq, kind, request_id, tool_call_id)` after removing `logicalId`
- `SessionAppendOnly`, `SessionReplayFromOffsetIsSuffix`, `HarnessSuspendReleasedOnlyByMatchingApproval`: keep them unchanged semantically, but rename any `"prompt_turn_started"` references to `"prompt_request_started"`

## Part 2: Stateright Bindings

Target files: new `verification/stateright/src/canonical_ids.rs`, update `verification/stateright/src/lib.rs`, update `verification/docs/refinement-matrix.md`.

### 2.1 Refinement mapping

The Stateright layer should add a `CanonicalIdsModel` and a `CanonicalIdsAction` enum. The model stays abstract, but every action must be derivable from an observed Rust transition.

Mapping table:

| Rust transition | Source | TLA+ action | Stateright action |
|---|---|---|---|
| prompt request enters the projector | `crates/fireline-harness/src/state_projector.rs:296-359` | `HarnessEmit(..., "prompt_request_started", ...)` | `AppendPromptRequestStarted { session_id, request_id }` |
| chunk notification becomes a row | `crates/fireline-harness/src/state_projector.rs:488-565` | `HarnessEmit(..., "chunk_appended", ...)` | `AppendChunk { session_id, request_id, tool_call_id }` |
| approval gate emits `permission_request` | `crates/fireline-harness/src/approval.rs:303-335` | `RequestApproval` | `EmitPermissionRequest { session_id, request_id }` |
| approval resolution is appended | `packages/client/src/events.ts:10-31`, `packages/client/src/agent.ts:52-64` | `ResolveApproval` | `ResolveApproval { session_id, request_id, allow }` |
| materializer or approval gate rebuilds from the stream | `crates/fireline-harness/src/approval.rs:207-299`, `crates/fireline-session/src/state_materializer.rs` | `ReplayFromOffset` | `ReplayFromOffset { session_id, offset }` |
| host wake/rebind after death | existing wake path in `managed_agents.tla`; runtime tests through managed-agent harness | `WakeReady` or `WakeStopped` | `ResumeOnHost { session_id, host_generation }` |

The refinement adapter should be a plain Rust normalizer:

- input: emitted `StateEnvelope`s, observed ACP requests/responses, and replay offsets from the live Rust components
- output: `CanonicalIdsAction`
- assertion: every live transition must normalize to exactly one TLA/Stateright action or be explicitly marked as infra-only and excluded from the agent-plane model

### 2.2 New properties

Add these property checks in `verification/stateright/src/canonical_ids.rs` and register them in `verification/stateright/src/lib.rs`:

1. `ConcurrentApprovalsRemainSessionScoped`
   - model two sessions with interleaved `EmitPermissionRequest` and `ResolveApproval`
   - assert a resolution for `(session_b, request_x)` cannot release `(session_a, request_x)`

2. `ReplayPreservesCanonicalIdentifiers`
   - allow replay from any offset `0..4`
   - after each replay, assert the model-level projections still satisfy:
     - `AgentLayerIdentifiersAreCanonical`
     - `InfrastructureAndAgentPlanesDisjoint`
     - `ApprovalKeyedByCanonicalRequestId`

3. `PromptRequestRefUniquePerSession`
   - important nuance: ACP `RequestId` is not globally unique, so the property is over the pair `(SessionId, RequestId)`
   - assert the model never contains two `prompt_request_started` entries for the same pair
   - if the implementation accidentally allows duplicate `(session_id, request_id)` prompt rows, this property must fail

Recommended unit-test names: `canonical_ids_model_checks_concurrent_approval_scoping`, `canonical_ids_model_checks_replay_preserves_identifier_invariants`, `canonical_ids_model_rejects_duplicate_prompt_request_ref_per_session`.

### 2.3 Counterexample shrinking

The checker should use BFS, not DFS:

```rust
let checker = CanonicalIdsModel::default().checker().spawn_bfs().join();
```

Minimization rules:

- keep the action alphabet to `{ AppendPromptRequestStarted, AppendChunk, EmitPermissionRequest, ResolveApproval, ReplayFromOffset, ResumeOnHost }`
- bound actions to the same small model as TLC: `<= 3` sessions, `<= 5` request ids, `<= 3` tool-call ids, `offset <= 4`
- render traces as tuples: `(session_id, request_id, tool_call_id?, action)`

Expected shrink target: 4 to 6 actions for approval-scoping failures, 2 actions for duplicate prompt refs.

## Part 3: Mechanical Audit Tooling

### 3.1 Rust compile-time type audit

Target files: new `verification/audit/Cargo.toml`, `verification/audit/build.rs`, `verification/audit/src/lib.rs`, `verification/audit/agent_layer_manifest.toml`.

Design:

1. Add a small workspace crate `verification/audit`.
2. In `build.rs`, parse selected Rust source files with `syn`.
3. Read `agent_layer_manifest.toml`, which lists the structs and fields that must use canonical ACP types.
4. Fail `cargo check` by `panic!` in `build.rs` if any listed field is typed as `String`, `Option<String>`, or a Fireline wrapper instead of:
   - `sacp::schema::SessionId`
   - `sacp::schema::RequestId`
   - `Option<sacp::schema::ToolCallId>`

Initial manifest entries should cover at least:

- `crates/fireline-session/src/lib.rs::SessionRecord.session_id`
- the agent-layer rows in `crates/fireline-harness/src/state_projector.rs`
- `crates/fireline-tools/src/peer/transport.rs::PeerCallResult.child_session_id`
- approval-path structs in `crates/fireline-harness/src/approval.rs`

Failing examples the audit must catch: `pub session_id: String`, `pub request_id: String`, `pub tool_call_id: Option<String>`.

### 3.2 TypeScript compile-time type audit

Target files: new `packages/state/test-types/acp-sdk-branded.d.ts`, `packages/state/test-types/canonical-identifiers.typecheck.ts`, `packages/state/tsconfig.canonical-ids.json`, and an optional mirror in `packages/client/test-types/`.

Important constraint:

- today the ACP TypeScript SDK does **not** export nominally branded ids; in `dist/schema/types.gen.d.ts`, `SessionId = string`, `ToolCallId = string`, and `RequestId = null | number | string`
- that means a direct `SessionId` vs `string` check is not mechanically meaningful

To make the audit real, the typecheck must use a test-only branded shim:

1. `acp-sdk-branded.d.ts` re-exports the ACP names with opaque brands:
   - `SessionId = string & { readonly __acpBrand: "SessionId" }`
   - `ToolCallId = string & { readonly __acpBrand: "ToolCallId" }`
   - `RequestId = null | (string & { readonly __acpBrand: "RequestIdString" }) | (number & { readonly __acpBrand: "RequestIdNumber" })`
2. `tsconfig.canonical-ids.json` path-maps `@agentclientprotocol/sdk` to that test-only file.
3. `canonical-identifiers.typecheck.ts` imports public row types from `packages/state/src/schema.ts` and public client signatures from `packages/client/src/agent.ts` and `packages/client/src/events.ts`.
4. Use helper types:

```ts
type Expect<T extends true> = T
type Equal<A, B> =
  (<T>() => T extends A ? 1 : 2) extends
  (<T>() => T extends B ? 1 : 2) ? true : false
```

Required assertions: `Expect<Equal<SessionRow['sessionId'], SessionId>>`, `Expect<Equal<PermissionRow['requestId'], RequestId>>`, `Expect<Equal<PermissionRow['toolCallId'], ToolCallId | undefined>>`, and the same shape for `appendApprovalResolved()` and `FirelineAgent.resolvePermission()`.

If any field is still plain `string`, this compile should fail.

### 3.3 Forbidden-identifier grep audit

Target file: new `verification/audit/tests/forbidden_identifiers.rs`.

Scope scanned:

- `crates/fireline-session/src/**`
- `crates/fireline-harness/src/state_projector.rs`
- `packages/state/src/schema.ts`

Forbidden tokens: `prompt_turn_id`, `trace_id`, `parent_prompt_turn_id`, `chunk_seq`, `chunk_id`, `logical_connection_id`, `edge_id`.

Rule:

- fail if any token appears in those paths
- allow matches only when the file contains the exact header annotation:
  - `fireline-verify: allow-legacy-agent-identifiers`
- the annotation is allowed only in explicitly named migration or deprecation modules

This test should use regex word boundaries so it does not trip on unrelated prose or substrings.

### 3.4 Plane-disjointness runtime audit

Target file: new `verification/audit/tests/plane_disjointness.rs`.

Test design:

1. Instantiate `fireline_harness::state_projector::StateProjector`.
2. Feed it mock ACP events that produce agent-layer rows.
3. Separately build mock infrastructure envelopes shaped like `hosts:tenant-*` state events.
4. Serialize the projected agent rows and assert they never contain:
   - `runtimeKey`
   - `runtimeId`
   - `nodeId`
   - `providerInstanceId`
5. Serialize the infrastructure rows and assert they never contain:
   - `sessionId`
   - `requestId`
   - `toolCallId`

Recommended test name: `projected_agent_rows_are_plane_disjoint_from_host_rows`.

## Part 4: Migration Dual-Read Verification

Target files: new `tests/acp_canonical_identifiers_migration.rs` and `tests/fixtures/acp_canonical_identifiers/`.

Fixtures:

- `legacy-only.ndjson`: historical `prompt_turn`, `chunk`, `permission`, and `child_session_edge` rows
- `dual-read-window.ndjson`: both legacy rows and new canonical rows for the same logical session
- `canonical-only.ndjson`: only `prompt_request`, canonical `permission`, and chunk rows without synthetic ids

### 4.1 Dual-read test

Test name:

- `dual_read_accepts_legacy_and_canonical_agent_rows_during_migration_window`

Assertions:

1. reading `legacy-only.ndjson` with the compatibility reader succeeds
2. reading `dual-read-window.ndjson` succeeds and exposes both:
   - legacy rows for backward compatibility
   - canonical rows as the preferred shape
3. no envelope is misclassified:
   - `prompt_turn` never parsed as `prompt_request`
   - `child_session_edge` never parsed as a canonical lineage row
   - canonical rows never fall back to legacy decoders

### 4.2 Cutover test

Test name:

- `cutover_rejects_legacy_agent_row_types_with_explicit_error`

Assertions:

1. turn off the migration reader with an explicit cutover flag
2. load `legacy-only.ndjson`
3. expect a hard failure, not silent fallback
4. error text should include the removed row type, for example:
   - `legacy agent-layer row type no longer supported: prompt_turn`

The cutover behavior must be loud because silent success would hide partial migrations.

## Part 5: End-to-End Scenarios

Target file: new `tests/acp_canonical_identifiers_e2e.rs`.

Use the existing harness support in `tests/support/managed_agent_suite.rs`.

### 5.1 Happy path: prompt, approval, resolve, replay

Test name:

- `canonical_ids_happy_path_prompt_approval_replay`

Preconditions:

- one sandbox
- approval middleware enabled
- state stream materializer running

Action sequence:

1. provision the sandbox
2. send a prompt that triggers approval
3. observe `permission_requested`
4. resolve it via the canonical request id
5. replay from offset `0`

Observable evidence:

- every session, permission, and chunk row uses only canonical ACP ids
- no row contains forbidden fields
- replay reconstructs the same pending/resolved approval state

Post-test audit:

- run the forbidden-identifier audit from Part 3

### 5.2 Cross-session peer call

Test name:

- `canonical_ids_peer_call_propagates_traceparent_without_bespoke_lineage_fields`

This replaces the old assertions in `tests/mesh_baseline.rs` that currently look for `promptTurnId`, `traceId`, `parentPromptTurnId`, and `child_session_edge`.

Preconditions:

- two agents discover each other
- peer-call path enabled

Action sequence:

1. agent A prompts agent B through the peer tool path
2. capture the outbound ACP envelope from A
3. capture the inbound ACP envelope and projected state rows on B

Observable evidence:

- outbound `_meta.traceparent` is present
- inbound `_meta.traceparent` is present
- the trace tree is coherent: same trace-id, inbound parent span matches outbound span
- neither session stream contains `traceId`, `parentPromptTurnId`, `logicalConnectionId`, or `child_session_edge`

Post-test audit:

- grep the peer modules and state schema for forbidden lineage tokens

### 5.3 Runtime death and resume mid-approval

Test name:

- `canonical_ids_runtime_death_resume_mid_approval_without_synthetic_bridge_ids`

Preconditions:

- host A owns the session
- prompt is blocked on approval

Action sequence:

1. send a prompt that triggers approval
2. kill host A before approval resolves
3. start host B and `session/load`
4. append `approval_resolved`
5. observe the session continue on B

Observable evidence:

- replay on B reconstructs the blocked approval from the canonical `RequestId`
- no `prompt_turn_id`, hashed approval id, or active-turn bridge is needed
- the resumed session continues after the resolution arrives via the stream

Post-test audit:

- grep and runtime assertions both show zero synthetic ids in the agent plane

## Appendix: Verification Gates

These gates are mandatory before each implementation phase lands.

| Phase from `acp-canonical-identifiers.md` | Required gate |
|---|---|
| Phase 0: ACP prerequisite | confirm ACP exposes `ToolCallId` where Fireline needs it; if not, block the refactor |
| Phase 1: canonical ref types | `ManagedAgentsCanonicalIds.cfg` passes TLC; Rust type audit crate exists and fails on a seeded `String` regression |
| Phase 1.5: replace plain-string ACP ids | Rust type audit passes; TS branded-shim typecheck passes; forbidden-identifier grep is green for touched files |
| Phase 2: dual-write canonical rows | dual-read fixture test passes; Stateright `ReplayPreservesCanonicalIdentifiers` passes |
| Phase 3: approval gate first | Stateright `ConcurrentApprovalsRemainSessionScoped` passes; happy-path E2E test passes |
| Phase 4: prompt/chunk/session projection | `ChunkOrderingFromStreamOffset` TLC invariant passes; plane-disjointness runtime audit passes |
| Phase 5: trace propagation and OTel | cross-session peer-call E2E test passes and replaces the old `mesh_baseline` synthetic-lineage assertions |
| Phase 6: remove synthetic-only structures | grep audit is zero in steady-state agent-plane files; runtime death/resume E2E passes |
| Phase 7: TS schema and query builders | TS branded-shim typecheck passes against `packages/state` and `packages/client`; migration compatibility tests still pass until cutover |
| Final cutover | legacy-row cutover test passes; all TLC, Stateright, audit, migration, and E2E gates are green in the same PR |

The refactor is not complete until all five layers are green simultaneously. A single passing happy-path integration test is not enough for this change.
