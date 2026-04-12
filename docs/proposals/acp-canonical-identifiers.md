# ACP Canonical Identifiers

> Status: proposal
> Date: 2026-04-12
> Scope: harness, session/state schema, peer lineage, approvals, verification, Platform SDK

## TL;DR

Fireline should stop inventing lineage identifiers and use ACP's canonical identifiers directly:

- `SessionId` is the only canonical session identity.
- `RequestId` is the canonical identity for JSON-RPC requests, including `session/prompt` and `request_permission`.
- `ToolCallId` is the canonical identity for tool invocations within a session.

There is no ACP `prompt_id` or `prompt_turn_id`. `prompt_turn_id`, `trace_id`, `parent_prompt_turn_id`, and the approval gate's hashed `request_id` are all Fireline inventions. They exist mostly because earlier layers could not see or propagate ACP identifiers at the right seam.

The proposal is:

1. Replace `prompt_turn_id` with a canonical prompt request ref: `(session_id, request_id)`.
2. Replace `trace_id` and `parent_prompt_turn_id` with a canonical parent invocation ref: `(parent_session_id, parent_request_id, parent_tool_call_id?)`.
3. Replace the approval gate's hashed `request_id` with the actual ACP JSON-RPC request id already carried by the permission flow.
4. Demote `logical_connection_id`, `chunk_id`, and `edge_id` from identity spines to derived storage keys.
5. Keep `host_key`, `runtimeId`, `node_id`, and `provider_instance_id` as Fireline infrastructure identifiers. ACP does not define host identity.

This is a prerequisite for generalizing `DurableSubscriber`: today the approval path already bakes synthetic identity into the one durable workflow Fireline got right.

## 1. Canonical ACP Identity Set

The relevant ACP identities already exist:

- `SessionId`: canonical ACP session identity. Source: ACP session setup spec.
  https://agentclientprotocol.com/protocol/session-setup#session-id
- `RequestId`: canonical JSON-RPC request identity. In the ACP TypeScript schema it is `null | number | string`.
  https://github.com/agentclientprotocol/typescript-sdk/blob/0d9436a12d8f3054b5dd0b2dd387dc1a8d880edd/src/schema/types.gen.ts#L3728
- `ToolCallId`: canonical tool invocation identity within a session.
  https://github.com/agentclientprotocol/typescript-sdk/blob/0d9436a12d8f3054b5dd0b2dd387dc1a8d880edd/src/schema/types.gen.ts#L5075

Fireline should standardize on three reference shapes:

```text
PromptRequestRef   = { session_id: SessionId, request_id: RequestId }
ToolInvocationRef  = { session_id: SessionId, tool_call_id: ToolCallId }
ParentCallRef      = {
  parent_session_id: SessionId,
  parent_request_id: RequestId,
  parent_tool_call_id?: ToolCallId,
}
```

Anything that cannot be expressed in those terms is either:

- transport-local bookkeeping, or
- real Fireline infrastructure identity, not ACP identity.

## 2. Synthetic Identifier Audit

### 2.1 Remove: synthetic lineage and approval ids

| Synthetic id | Where minted | Current consumers | Why it exists | Canonical replacement | Verdict |
|---|---|---|---|---|---|
| `prompt_turn_id` / `promptTurnId` | `crates/fireline-harness/src/state_projector.rs:302-358`, `:568-574` | `state_projector.rs`; `crates/fireline-session/src/active_turn_index.rs`; `crates/fireline-tools/src/peer/lookup.rs`, `peer/mcp_server.rs`, `peer/transport.rs`; `crates/fireline-orchestration/src/child_session_edge.rs`; `packages/state/src/schema.ts:14-20, 37-41, 55-67, 70-80, 117-125`; session-turn and turn-chunk query builders | Earlier layers could not rely on the ACP prompt request id everywhere, so Fireline minted a durable per-turn surrogate and treated it as the lineage spine | `PromptRequestRef = (session_id, request_id)` | Delete |
| `trace_id` / `traceId` | `state_projector.rs:304-329`; inherited from `_meta.fireline.traceId` in `state_projector.rs:635-699`; approval escape hatch in `crates/fireline-harness/src/approval.rs:189-205, 436-446` | `approval.rs`; `active_turn_index.rs`; `peer/lookup.rs`; `peer/mcp_server.rs:217-220`; `peer/transport.rs:167-185`; `child_session_edge.rs:22-34, 53-70`; `packages/state/src/schema.ts:19, 99, 108` | Fireline needed a stable cross-session lineage token before it threaded canonical parent refs through peer calls | `ParentCallRef` and, where needed, `PromptRequestRef` | Delete |
| `parent_prompt_turn_id` / `parentPromptTurnId` | parsed from `_meta.fireline.parentPromptTurnId` in `state_projector.rs:670-699`; written into prompt/session rows in `state_projector.rs:323-329, 403-410` | `crates/fireline-session/src/lib.rs:37-53`; `session_index.rs`; `peer/lookup.rs:11-18`; `peer/mcp_server.rs:169-183`; `peer/transport.rs:171-175`; `child_session_edge.rs:26-33, 78-89`; `packages/state/src/schema.ts:20, 100, 111` | Fireline lacked a structured parent request reference, so it carried only the parent's synthetic prompt-turn id | `ParentCallRef.parent_request_id` plus optional `parent_tool_call_id` | Delete |
| approval `request_id` hash | `crates/fireline-harness/src/approval.rs:183-205`, emitted at `:303-335` | `approval.rs:207-420, 520-541`; `packages/client/src/events.ts:10-32`; `packages/client/src/agent.ts:52-64`; `packages/state/src/schema.ts:55-68` where `requestId` and `jsonrpcId` coexist | The approval gate runs before prompt projection and could not see or trust the canonical id path, so it hashed session + policy + prompt identity | ACP `RequestId` from the actual `request_permission` JSON-RPC message; optionally paired with `ToolCallId` | Delete |

### 2.2 Demote: derived storage keys, not canonical identity

| Synthetic id | Where minted | Current consumers | Clean replacement | Verdict |
|---|---|---|---|---|
| `logical_connection_id` / `logicalConnectionId` | `crates/fireline-harness/src/routes_acp.rs:70-89` | `state_projector.rs`; `crates/fireline-session/src/lib.rs:37-53`; `packages/state/src/schema.ts:4-12, 14-17, 37-47, 55-68, 70-80, 91-104, 117-125`; connection-turn queries | Keep only as optional transport telemetry. It should not key prompt, approval, chunk, or session lineage. In a per-session stream design it becomes largely redundant. | Demote |
| `chunk_id` / `chunkId` | `state_projector.rs:544-565` | `packages/state/src/schema.ts:117-125`; `packages/state/src/collections/turn-chunks.ts` | Use a derived row key like `{session_id}:{request_id}:{seq}`. The semantic identity of a chunk is "chunk `seq` within prompt request `(session_id, request_id)`". | Demote |
| `edge_id` / `edgeId` | `crates/fireline-orchestration/src/child_session_edge.rs:52-96` | `child_session_edge.rs`; `packages/state/src/schema.ts:106-115` | Use a derived storage key over canonical refs, e.g. `{parent_session_id}:{parent_request_id}:{child_session_id}` with `parent_tool_call_id` when present. | Demote |

### 2.3 Keep: real Fireline infrastructure ids, not ACP ids

| Id | Where minted / defined | Why it stays |
|---|---|---|
| `child_session_id` / `childSessionId` | returned from peer call in `crates/fireline-tools/src/peer/transport.rs:111-149` and stored in `peer/lookup.rs:11-18` | This is already an ACP `SessionId`. Keep it. |
| `host_key` / `runtimeKey` | minted in `src/main.rs:344-345`, carried by `crates/fireline-session/src/host_identity.rs:82-97` | ACP has no host identity. This is Fireline's cross-restart runtime identity. |
| `host_id` / `runtimeId` | minted in `crates/fireline-host/src/bootstrap.rs:136-137`, carried by `host_identity.rs:82-110` | ACP has no per-process host instance id. This is valid infrastructure identity. |
| `node_id` / `nodeId` | minted in `src/main.rs:345`, carried by `host_identity.rs:82-110` | ACP has no cluster/node identity. Keep. |
| `provider_instance_id` / `providerInstanceId` | assigned in `src/main.rs:370`, carried by `host_identity.rs:82-110` | Provider runtime/container identity is outside ACP. Keep. |

### 2.4 Out of scope: generated object ids with no ACP analogue

The grep pass also surfaced generated ids such as:

- `terminalId` in `packages/state/src/schema.ts:70-81`
- `blob_key` in `src/main.rs:291-314`
- sandbox/runtime ids minted by providers in `crates/fireline-sandbox/src/providers/local_subprocess.rs:232` and `docker.rs:192`

These are not good candidates for ACP replacement because they are not trying to model ACP session/request/tool-call identity. They may still deserve cleanup or renaming, but they are not the synthetic lineage problem this proposal is solving.

## 3. Synthetic-Only Structures That Should Disappear

### 3.1 `TraceCorrelationState`

Defined in `crates/fireline-harness/src/state_projector.rs:166-174`.

What exists only because of synthetic ids:

- `prompt_request_to_turn`: needed only because the durable row key is not the ACP request id.
- `session_active_turn`: needed because peer lineage tracks a synthetic turn id rather than a canonical parent request ref.
- `chunk_seq`: valid ordering state, but it is keyed by synthetic `prompt_turn_id`.
- `turn_counter`: exists only to mint `prompt_turn_id`.

After the change:

- `turn_counter` disappears.
- `prompt_request_to_turn` disappears; the prompt row is already keyed by `(session_id, request_id)`.
- `session_active_turn` either disappears completely or becomes `session_active_request: SessionId -> RequestId` as a temporary bridge.
- `chunk_seq` survives only if chunk ordering must still be synthesized, but it is keyed by `PromptRequestRef`, not `prompt_turn_id`.

### 3.2 `InheritedLineage`

Defined in `state_projector.rs:177-180`.

Current shape:

```rust
struct InheritedLineage {
    trace_id: Option<String>,
    parent_prompt_turn_id: Option<String>,
}
```

Replacement:

```rust
struct ParentCallRef {
    parent_session_id: String,
    parent_request_id: String,
    parent_tool_call_id: Option<String>,
}
```

This still allows child-session lineage, but it is canonical and structured.

### 3.3 `ActiveTurnIndex`

`crates/fireline-session/src/active_turn_index.rs` is 250 lines of state that exists almost entirely to bridge from session id to Fireline's invented prompt-turn lineage.

Today it returns:

- `prompt_turn_id`
- `trace_id`

to peer code at `crates/fireline-tools/src/peer/mcp_server.rs:210-220`.

Clean end state:

- tools receive `ToolCallId` directly from ACP tool execution context
- prompt-side lineage uses `RequestId`
- peer handoff carries `ParentCallRef`

If ACP tool context still does not expose `ToolCallId`, keep only a temporary `ActiveRequestIndex` keyed by session id and returning `PromptRequestRef`. Do not preserve `trace_id` or `prompt_turn_id`.

### 3.4 Session rows carrying lineage baggage

`crates/fireline-session/src/lib.rs:37-53` stores `trace_id` and `parent_prompt_turn_id` on `SessionRecord`.

That is not session identity. It is leaked call-lineage state.

`SessionRecord` should keep:

- `session_id`
- `host_key`
- `host_id`
- `node_id`
- `supports_load_session`
- timestamps

and drop:

- `trace_id`
- `parent_prompt_turn_id`
- eventually `logical_connection_id` unless there is a concrete observability need

## 4. Proposed Canonical Replacements

### 4.1 Prompt lifecycle

Current:

```text
prompt_turn_id = "{host_id}:{logical_connection_id}:{counter}"
```

Proposed:

```text
prompt_request_ref = { session_id, request_id }
```

Implications:

- `prompt_turn` should be renamed to `prompt_request`, or at minimum re-keyed by canonical request identity.
- the row's durable key becomes a derived string from canonical parts, for example `{session_id}:{request_id}`
- `pending_request.prompt_turn_id` disappears
- `chunk.promptTurnId` becomes `chunk.requestId` plus `chunk.sessionId`

### 4.2 Approval lifecycle

Current:

- synthetic `Permission.requestId`
- canonical `Permission.jsonrpcId`
- optional `Permission.toolCallId`

That schema is already admitting the problem: it has both the fake id and the real one.

Proposed:

```text
PermissionRef = { session_id, request_id }
PermissionRow = {
  session_id,
  request_id,        // canonical JSON-RPC request id
  tool_call_id?,     // canonical ACP tool call id when present
  state,
  outcome,
  ...
}
```

The approval gate should emit and wait on the actual JSON-RPC `request_permission` id, not a SHA256 of prompt text.

`packages/client/src/events.ts:10-32` and `packages/client/src/agent.ts:52-64` then become correct as written: `requestId` means ACP `RequestId`, not "Fireline approval hash".

### 4.3 Peer lineage and child-session edges

Current parent lineage:

```text
trace_id + parent_prompt_turn_id
```

Proposed parent lineage:

```text
ParentCallRef {
  parent_session_id,
  parent_request_id,
  parent_tool_call_id?,
}
```

Current child-session edge:

- `trace_id`
- `parent_host_id`
- `parent_session_id`
- `parent_prompt_turn_id`
- `child_host_id`
- `child_session_id`

Proposed child-session edge:

- `parent_host_id` (infra identity, keep)
- `parent_session_id` (ACP)
- `parent_request_id` (ACP)
- `parent_tool_call_id?` (ACP)
- `child_host_id` (infra identity, keep)
- `child_session_id` (ACP)

`trace_id` and `parent_prompt_turn_id` disappear.

### 4.4 Chunks

Current chunk identity:

- synthetic `chunk_id`
- synthetic foreign key `prompt_turn_id`

Proposed:

```text
ChunkRow {
  session_id,
  request_id,
  tool_call_id?,   // if a chunk is attributable to a tool call
  seq,
  type,
  content,
  created_at,
}
```

Durable key can be derived as `{session_id}:{request_id}:{seq}`.

## 5. State Schema Changes

The TypeScript schema at `packages/state/src/schema.ts:14-125` should change as follows.

### 5.1 Prompt rows

Current:

- `promptTurnId`
- `requestId`
- `traceId`
- `parentPromptTurnId`

Proposed:

- `sessionId`
- `requestId`
- `parentSessionId?`
- `parentRequestId?`
- `parentToolCallId?`

Prefer renaming collection/entity type from `promptTurns` / `prompt_turn` to `promptRequests` / `prompt_request`. There is no ACP "turn" identifier to preserve.

### 5.2 Permission rows

Current:

- `requestId`
- `jsonrpcId`
- `promptTurnId`
- `toolCallId?`

Proposed:

- `requestId` becomes the canonical JSON-RPC id
- remove `jsonrpcId`
- remove `promptTurnId`
- keep `toolCallId?`
- optionally add `parentRequestId?` only if there is a proven need

### 5.3 Session rows

Current:

- `traceId?`
- `parentPromptTurnId?`
- `logicalConnectionId`

Proposed:

- remove `traceId`
- remove `parentPromptTurnId`
- make `logicalConnectionId` optional telemetry or remove it entirely

### 5.4 Child-session edges

Current:

- `edgeId`
- `traceId`
- `parentPromptTurnId`

Proposed:

- remove `traceId`
- replace `parentPromptTurnId` with `parentRequestId`
- add `parentToolCallId?`
- treat `edgeId` as a derived storage key, not semantic identity

### 5.5 Chunks

Current:

- `chunkId`
- `promptTurnId`

Proposed:

- `sessionId`
- `requestId`
- `toolCallId?`
- `seq`

## 6. Durable-Streams Sessions

Durable Streams does not appear to provide a special protocol-level "ACP session" primitive. The integration doc describes a durable sessions pattern:

- one JSON-mode stream per session
- session structure lives in the event payload
- the stream is replayed and tailed via SSE

Source:
https://thesampaton.github.io/durable-streams-rust-server/integration/sessions.html

So the clean mapping is architectural rather than built into the DS wire format:

```text
ACP SessionId 1:1 Fireline session stream name
```

For example:

```text
state/session/{session_id}
```

What collapses if Fireline adopts that mapping:

- approval replay no longer scans a tenant-wide stream and filters by `sessionId`; it reads that session's stream only
- prompt/chunk/session lifecycle rows no longer need `sessionId` as a secondary filter over a shared stream; it is implicit in the stream
- `SessionIndex` can shrink from "global shared-stream materializer" to "per-session materializer + optional cross-session registry"
- durable subscribers become naturally session-scoped

What does not collapse:

- host discovery remains tenant-wide or host-wide, not per session
- cross-session edges still need a shared relationship view or dual-write into both session streams
- infrastructure ids (`host_key`, `runtimeId`, `node_id`) still exist because DS sessions do not replace host identity

Recommendation:

- use per-session streams for ACP session truth
- keep a separate tenant-wide host/discovery stream
- do not force host discovery or runtime inventory into per-session streams

## 7. Verification Impact

`verification/spec/managed_agents.tla` is already closer to the desired design than the runtime code.

Relevant references:

- event kinds: `verification/spec/managed_agents.tla:36-48`
- request-scoped approval state: `:182-197`
- approval transitions: `:361-445`
- invariants over released approvals: `:785-787`

Notably:

- the spec uses `RequestIds`
- it does not carry `prompt_turn_id`
- it already models approval correctness in request terms

Required updates:

1. rename `"prompt_turn_started"` to `"prompt_request_started"` or equivalent
2. clarify that `RequestIds` are ACP JSON-RPC request ids
3. if peer lineage is added to the model, model it in terms of `ParentCallRef`, not trace ids
4. keep `HarnessSuspendReleasedOnlyByMatchingApproval`; it already states the right invariant once `req` is canonical

This is good news: the TLA model mostly needs alignment and naming cleanup, not conceptual redesign.

## 8. Downstream Dependencies

### 8.1 `DurableSubscriber`

`docs/proposals/durable-subscriber.md:9-20, 28-35` generalizes the approval gate. That proposal should not bake in the approval gate's current synthetic `request_id` hash.

`DurableSubscriber` should standardize on:

- canonical request ids
- canonical session ids
- optional tool call ids

Otherwise Fireline will generalize the wrong abstraction.

### 8.2 Platform SDK

`docs/proposals/platform-sdk-api-design.md:13-15, 27-45, 105-117` already wants:

- `fireline.db()` as the global state entry point
- `agent.resolvePermission(sessionId, requestId, ...)`

That API becomes unambiguous only after `requestId` means ACP `RequestId`, not Fireline's hash.

## 9. Implementation Plan

### Phase 1: introduce canonical ref types

Add shared Rust and TypeScript types:

- `PromptRequestRef`
- `ToolInvocationRef`
- `ParentCallRef`

Do this before changing storage so all new code speaks the same language.

### Phase 2: dual-write canonical rows

Do not mutate old entity shapes in place while readers still replay historical streams.

Prefer one of:

- new entity types, e.g. `prompt_request`, `permission_v2`, `child_session_edge_v2`, or
- backward-compatible readers that accept both old and new row shapes

The safer cut is new entity types. Historical synthetic events already in append-only streams cannot be rewritten.

### Phase 3: approval gate first

Change `crates/fireline-harness/src/approval.rs` to emit and wait on the real JSON-RPC permission request id.

Why first:

- it is the smallest isolated win
- it unblocks `DurableSubscriber`
- it makes the Platform SDK's `resolvePermission(sessionId, requestId)` semantically correct

### Phase 4: prompt/chunk/session projection

Change `StateProjector` so prompt lifecycle rows are keyed by canonical prompt request identity.

This removes:

- `next_prompt_turn_id()`
- `turn_counter`
- `prompt_request_to_turn`

and rekeys chunk ordering to canonical prompt request refs.

### Phase 5: peer lineage

Update:

- `crates/fireline-tools/src/peer/lookup.rs`
- `peer/mcp_server.rs`
- `peer/transport.rs`
- `crates/fireline-orchestration/src/child_session_edge.rs`

to carry `ParentCallRef`.

If current ACP tool context does not expose `ToolCallId`, add that seam instead of preserving `trace_id`.

### Phase 6: remove synthetic-only structures

Delete or collapse:

- `TraceCorrelationState` fields that only exist for synthetic ids
- `InheritedLineage`
- `ActiveTurnIndex` or reduce it to a temporary `ActiveRequestIndex`
- `trace_id` and `parent_prompt_turn_id` from `SessionRecord`

### Phase 7: TS schema and query builders

Update:

- `packages/state/src/schema.ts`
- session-turn / turn-chunk / active-turn collections
- fixtures and tests

to canonical request refs.

## 10. Tests and Compatibility

Tests that must change:

- `crates/fireline-harness/src/approval.rs` tests around `approval_request_id`
- `crates/fireline-session/src/active_turn_index.rs`
- `crates/fireline-session/src/session_index.rs`
- `packages/state/test/schema.test.ts`
- `packages/state/test/collections.test.ts`
- `packages/state/test/fixtures/rust-state-producer.ndjson`
- peer lineage tests around child-session edges

Backward-compatibility concerns:

- existing streams already contain synthetic `prompt_turn`, `permission`, `child_session_edge`, and `chunk` rows
- append-only history means we cannot rewrite those rows
- readers must either support both shapes or switch to versioned entity types

Recommendation:

- dual-read for one migration window
- dual-write only if necessary
- then stop writing synthetic rows and delete their readers

## 11. Final Recommendation

Fireline should draw a hard boundary:

- ACP identifiers are the only canonical identifiers for session, request, and tool-call lineage.
- Fireline infrastructure identifiers remain for hosts and providers only.
- Derived storage keys stay derived and stop leaking into the public domain model.

Concretely:

- delete `prompt_turn_id`
- delete `trace_id`
- delete `parent_prompt_turn_id`
- delete the approval gate's hashed `request_id`
- demote `logical_connection_id`, `chunk_id`, and `edge_id`
- keep `SessionId`, `RequestId`, `ToolCallId`, `host_key`, `runtimeId`, `node_id`, and `provider_instance_id`

If Fireline does not make this cut before `DurableSubscriber`, it will freeze old accidental identities into the next layer of generic abstractions.
