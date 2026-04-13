# QA: Canonical-ID TLC Vacuity Check (2026-04-12)

Scope: `verification/spec/managed_agents.tla` against `verification/spec/ManagedAgentsCanonicalIds.cfg`, plus temporary witness probes over the same `Next` relation to determine whether the checked invariants are actually exercised.

## Commands Run

Baseline run:

```sh
/opt/homebrew/opt/openjdk/bin/java \
  -cp /tmp/fireline-tla/tla2tools.jar \
  tlc2.TLC verification/spec/managed_agents.tla \
  -config verification/spec/ManagedAgentsCanonicalIds.cfg \
  -metadir /tmp/fireline-tla/qa/metadir-ManagedAgentsCanonicalIds
```

Targeted witness runs used a temporary copy of the spec under `/tmp/fireline-tla/qa/spec/managed_agents.tla` with probe operators only. No repo spec changes were required for this QA pass.

## Baseline Result

`verification/spec/ManagedAgentsCanonicalIds.cfg` currently checks only the five canonical-id invariants at [ManagedAgentsCanonicalIds.cfg](../../verification/spec/ManagedAgentsCanonicalIds.cfg). The checked-in model is narrow:

- `Sessions = {"s1"}`
- `RequestIds = {"r1"}`
- `ToolCallIds = {"tc1"}`
- `ProducerSeqs = {0, 1}`

That is enough to exercise the five canonical-id invariants, but not enough to say anything meaningful about true cross-session concurrency.

Baseline TLC result:

```text
Model checking completed. No error has been found.
1534928 states generated, 186314 distinct states found, 0 states left on queue.
The depth of the complete state graph search is 12.
Finished in 20s at (2026-04-12 15:52:05)
```

## Verdict Table

| Check | Source | Verdict | Evidence | Notes |
| --- | --- | --- | --- | --- |
| `AgentLayerIdentifiersAreCanonical` | `ManagedAgentsCanonicalIds.cfg` + witness probe | Pass, non-vacuous | `NoAgentRows` violated at State 2 after `RequestApproval`; `NoResolvedApprovalRows` violated at State 3; `NoMultipleChunksSameRequest` violated at State 4 | `AgentRows` is not empty in reachable states. |
| `InfrastructureAndAgentPlanesDisjoint` | `ManagedAgentsCanonicalIds.cfg` + witness probe | Pass, non-vacuous | `NoInfrastructureRows` violated at State 2 after `ProvisionRuntime` | Both agent-plane and infra-plane rows are reachable. |
| `CrossSessionLineageIsOutOfBand` | `ManagedAgentsCanonicalIds.cfg` + witness probe | Pass, non-vacuous | `NoTraceContextPropagation` violated at State 3 with `traceparent = "tp1"`, `tracestate = "ts1"`, `baggage = "bg1"` | The trace-context side channel is exercised; the invariant is not just quantifying over defaults. |
| `ChunkOrderingFromStreamOffset` | `ManagedAgentsCanonicalIds.cfg` + witness probe | Pass, non-vacuous | `NoMultipleChunksSameRequest` violated at State 4 with two `chunk_appended` rows for request `r1` | The ordering clause is exercised on a multi-chunk request, not just a singleton row. |
| `ApprovalKeyedByCanonicalRequestId` | `ManagedAgentsCanonicalIds.cfg` + witness probe | Pass, non-vacuous | `NoResolvedApprovalRows` violated at State 3 with `permission_requested` then `approval_resolved`, both keyed by `request_id = "r1"` | Approval/request propagation is exercised on real rows. |
| `SessionDurableAcrossRuntimeDeath` | direct TLC run of the existing invariant | Pass, non-vacuous | `NoStoppedRuntimeWithSnapshot` violated at State 4 after `stop_runtime`; direct run of `SessionDurableAcrossRuntimeDeath` also completed with no error | Not checked by `ManagedAgentsCanonicalIds.cfg`, but reachable and meaningful under the same `Next`. |
| `ConcurrentApprovalsRemainSessionScoped` | temporary QA invariant over widened model | Fail, non-vacuous | Widened model hits a counterexample at State 3 when both sessions request `r1` and `pendingApprovals` is overwritten | This is the one meaningful red flag. The checked-in cfg is too narrow to expose it. |

## Evidence

### 1. Agent rows are reachable

The minimal probe `NoAgentRows == ~ (AgentRows # {})` fails immediately:

```text
Error: Invariant NoAgentRows is violated.
State 2: <RequestApproval ...>
/\ sessionLog = [s1 |-> <<[... session_id |-> "s1", request_id |-> "r1",
                           tool_call_id |-> "tc1", kind |-> "permission_requested"]>>]
/\ pendingApprovals = [r1 |-> [sessionId |-> "s1", toolCallId |-> "tc1", state |-> "pending"]]
/\ blockedRequests = [s1 |-> "r1"]
```

This closes the vacuity concern for `AgentLayerIdentifiersAreCanonical`: `AgentRows` is not empty under the checked-in config.

### 2. Infrastructure rows are reachable

The minimal probe `NoInfrastructureRows` fails at the first provision step:

```text
Error: Invariant NoInfrastructureRows is violated.
State 2: <ProvisionRuntime ...>
/\ runtimeIndex = [ rk1 |->
      [ status |-> "ready", runtimeId |-> "rid0",
        nodeId |-> "n1", providerInstanceId |-> "p1",
        specPresent |-> TRUE, boundSessions |-> {"s1"} ] ]
/\ reachable = [rk1 |-> TRUE]
```

That makes `InfrastructureAndAgentPlanesDisjoint` non-vacuous on both sides of the split.

### 3. Trace context is actually propagated

`CrossSessionLineageIsOutOfBand` is not just quantifying over default placeholders. The probe `NoTraceContextPropagation` fails after `prompt_request_started`:

```text
Error: Invariant NoTraceContextPropagation is violated.
State 3: <HarnessEmit ...>
/\ sessionLog = [s1 |-> <<[... request_id |-> "r1",
                           tool_call_id |-> "no_tool_call",
                           kind |-> "prompt_request_started"]>>]
/\ traceContext = [ s1 |->
      [ r1 |->
          [traceparent |-> "tp1", tracestate |-> "ts1", baggage |-> "bg1"] ] ]
```

### 4. Multi-chunk ordering is exercised

`ChunkOrderingFromStreamOffset` is also non-vacuous. The probe `NoMultipleChunksSameRequest` fails with two chunks for the same request:

```text
Error: Invariant NoMultipleChunksSameRequest is violated.
State 4: <HarnessEmit ...>
/\ sessionLog = [s1 |-> <<[... seq |-> 0, request_id |-> "r1",
                           tool_call_id |-> "tc1", kind |-> "chunk_appended"],
                         [... seq |-> 1, request_id |-> "r1",
                           tool_call_id |-> "tc1", kind |-> "chunk_appended"]>>]
```

### 5. Approval rows are exercised with canonical request ids

`ApprovalKeyedByCanonicalRequestId` is exercised by a real request/resolve path:

```text
Error: Invariant NoResolvedApprovalRows is violated.
State 3: <ResolveApproval ...>
/\ sessionLog = [s1 |-> <<[... request_id |-> "r1",
                           tool_call_id |-> "tc1", kind |-> "permission_requested"],
                         [... request_id |-> "r1",
                           tool_call_id |-> "tc1", kind |-> "approval_resolved"]>>]
/\ approvalHistory = [r1 |-> <<"allow">>]
/\ pendingApprovals = [r1 |-> [sessionId |-> "s1", toolCallId |-> "tc1", state |-> "resolved_allow"]]
```

### 6. Runtime death antecedent is reachable

`SessionDurableAcrossRuntimeDeath` is not checked by the canonical cfg, but it is non-vacuous under the same `Next`. The direct invariant run passed:

```text
Model checking completed. No error has been found.
1534928 states generated, 186314 distinct states found, 0 states left on queue.
The depth of the complete state graph search is 12.
Finished in 16s at (2026-04-12 15:49:11)
```

Its antecedent is exercised by the `NoStoppedRuntimeWithSnapshot` probe:

```text
Error: Invariant NoStoppedRuntimeWithSnapshot is violated.
State 4: <StopRuntime ...>
/\ stopSnapshot = [rk1 |-> [s1 |-> <<[... kind |-> "session_created"]>>]]
/\ runtimeIndex = [ rk1 |->
      [ status |-> "stopped", runtimeId |-> "rid0", ... ] ]
```

## Concurrent Approvals: Not Vacuous, But Currently Failing Under Widened Coverage

This is the only check that did not end in a clean pass.

The checked-in config cannot witness it because [ManagedAgentsCanonicalIds.cfg](../../verification/spec/ManagedAgentsCanonicalIds.cfg) fixes `Sessions = {"s1"}` and `RequestIds = {"r1"}`. I widened only the temporary QA config to:

- `Sessions = {"s1", "s2"}`
- `RequestIds = {"r1", "r2"}`
- `RuntimeIds = {"rid0", "rid1"}`
- `ProducerSeqs = {0, 1, 2}`

Under that widened model, a healthy concurrent state is reachable:

```text
Error: Invariant NoTwoApprovalsInFlight is violated.
State 3: <RequestApproval ...>
/\ pendingApprovals = [ r1 |-> [sessionId |-> "s1", toolCallId |-> "tc1", state |-> "pending"],
                        r2 |-> [sessionId |-> "s2", toolCallId |-> "tc1", state |-> "pending"] ]
/\ blockedRequests = [s1 |-> "r1", s2 |-> "r2"]
```

So the model can represent two in-flight approvals that remain isolated.

However, the stricter QA invariant `ConcurrentApprovalsRemainSessionScoped` fails immediately when both sessions reuse the same canonical request id:

```text
Error: Invariant ConcurrentApprovalsRemainSessionScoped is violated.
State 3: <RequestApproval ...>
/\ sessionLog = [s1 |-> <<[... request_id |-> "r1", kind |-> "permission_requested"]>>,
                 s2 |-> <<[... request_id |-> "r1", kind |-> "permission_requested"]>>]
/\ pendingApprovals = [ r1 |-> [sessionId |-> "s2", toolCallId |-> "tc1", state |-> "pending"],
                        r2 |-> [sessionId |-> "s1", toolCallId |-> "no_tool_call", state |-> "none"] ]
/\ blockedRequests = [s1 |-> "r1", s2 |-> "r1"]
```

That is a real model hole, not vacuity:

- the approval map is keyed only by `RequestId`
- the model currently permits the same `RequestId` to appear in two different sessions
- the second request overwrites the first session's pending approval state

## Conclusion

### What is closed

The five checked-in canonical-id invariants in `ManagedAgentsCanonicalIds.cfg` are all:

- passing
- non-vacuous under the current checked-in model

`SessionDurableAcrossRuntimeDeath` also passes and is non-vacuous under the same `Next` relation.

### What remains open

`ConcurrentApprovalsRemainSessionScoped` is not a checked-in TLA invariant today, and when modeled over a widened configuration it fails. That means the current abstract state machine still needs one of these before Phase 3 can safely claim concurrent approval isolation:

1. enforce global uniqueness of `RequestId` across all sessions in the model, or
2. re-key approval state by `(session_id, request_id)` instead of `request_id` alone.

Without one of those, the current spec allows a cross-session approval collision even though the five narrower canonical-id invariants all pass.

## R1 Addendum: Checked-In Widened Re-Run For `ConcurrentApprovalsRemainSessionScoped`

The earlier widened evidence in this review was gathered from a temporary QA-only config. For `mono-c80`, that scenario is now formalized in the checked-in spec surface:

- invariant operator added to `verification/spec/managed_agents.tla`:
  `ConcurrentApprovalsRemainSessionScoped`
- focused sibling cfg added at
  [ManagedAgentsConcurrentApprovals.cfg](../../verification/spec/ManagedAgentsConcurrentApprovals.cfg)

That cfg intentionally widens the model to the smallest cross-session collision case:

- `Sessions = {"s1", "s2"}`
- `RequestIds = {"r1"}`
- both sessions may issue `RequestApproval(..., "r1", ...)`

Command rerun:

```sh
/opt/homebrew/opt/openjdk/bin/java \
  -cp /tmp/fireline-tla/tla2tools.jar \
  tlc2.TLC verification/spec/managed_agents.tla \
  -config verification/spec/ManagedAgentsConcurrentApprovals.cfg \
  -metadir /tmp/fireline-tla/qa/metadir-ManagedAgentsConcurrentApprovals
```

Result:

```text
Error: Invariant ConcurrentApprovalsRemainSessionScoped is violated.
...
State 3: <RequestApproval ...>
/\ sessionLog = [s1 |-> <<[... request_id |-> "r1", kind |-> "permission_requested"]>>,
                 s2 |-> <<[... request_id |-> "r1", kind |-> "permission_requested"]>>]
/\ pendingApprovals = [r1 |-> [sessionId |-> "s2", toolCallId |-> "tc1", state |-> "pending"]]
/\ blockedRequests = [s1 |-> "r1", s2 |-> "r1"]
...
110 states generated, 95 distinct states found, 89 states left on queue.
The depth of the complete state graph search is 3.
Finished in 00s
```

This closes the vacuity concern for the concurrent-approval case:

- the bad cross-session state is reachable under a checked-in config
- the invariant fails immediately and therefore is not vacuous
- the failure is specifically caused by `pendingApprovals` being keyed only by `RequestId`

Why this proves `(session_id, request_id)` keying is required:

- after `s1` requests approval for `r1`, the model stores
  `pendingApprovals["r1"].sessionId = "s1"`
- after `s2` requests approval for the same `r1`, the second write overwrites that slot with
  `pendingApprovals["r1"].sessionId = "s2"`
- both `blockedRequests["s1"]` and `blockedRequests["s2"]` still point at `"r1"`
- there is no way for a `RequestId -> PendingApproval` map to represent both pending approvals simultaneously

So the widened re-run confirms the architect's earlier read: `ConcurrentApprovalsRemainSessionScoped` is meaningfully exercised, and it only survives if the abstract approval state is re-keyed by `(session_id, request_id)` or if the model enforces global `RequestId` uniqueness across sessions.
