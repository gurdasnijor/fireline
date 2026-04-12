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
2. Replace `trace_id` and `parent_prompt_turn_id` with ACP-native W3C Trace Context propagation in `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage`.
3. Replace the approval gate's hashed `request_id` with the actual ACP JSON-RPC request id already carried by the permission flow.
4. Delete `logical_connection_id`, `chunk_id`, and `chunk_seq` from agent-layer state; chunk ordering comes from durable-streams offsets.
5. Delete the bespoke `child_session_edge` lineage table instead of renaming it.
6. Keep `host_key`, `runtimeId`, `node_id`, and `provider_instance_id` only in the infrastructure plane. ACP does not define host identity.

This is a prerequisite for generalizing `DurableSubscriber`: today the approval path already bakes synthetic identity into the one durable workflow Fireline got right.

## Acceptance Criterion

> No usage of any synthetic or out-of-band identifiers, or any mechanism for stitching together agent/session/prompt/tool graphs, OUTSIDE of what is provided by the ACP schema: https://agentclientprotocol.com/protocol/schema

This is the governing bar for every design choice in this proposal.

Allowed:

- identifiers defined in the ACP schema, including `SessionId`, `RequestId`, `ToolCallId`, and other canonical schema types
- ACP `_meta`, including reserved W3C Trace Context keys `traceparent`, `tracestate`, and `baggage`, because `_meta` is an explicit ACP extensibility surface
- derived storage keys that are pure concatenations of canonical ACP identifiers

Rejected:

- Fireline-minted ids used to stitch agent/session/prompt/tool graphs
- bespoke lineage or edge tables independent of ACP identifiers and `_meta`
- any parallel Fireline trace id or graph-stitching key
- hash-, UUID-, or counter-minted ids used as semantic graph identities

Explicitly allowed with justification:

- infrastructure ids used strictly for host/runtime/provider lookup such as `host_key`, `runtimeId`, `node_id`, and `provider_instance_id`
- derived row keys such as `{session_id}:{request_id}:{seq}` when they are documented as storage conveniences, not new identities

## Plane Separation

Fireline state splits into two disjoint planes with a hard boundary.

- Agent-layer plane: session, prompt, chunk, permission, and tool-call rows. These contain only ACP-schema identifiers and ACP-schema fields. No Fireline-minted ids of any kind. This plane lives in per-session durable streams such as `state/session/{session_id}`.
- Infrastructure-layer plane: host, sandbox, provider, and node rows. These contain Fireline-minted infrastructure ids and external provider ids. This plane lives in separate infra streams such as `hosts:tenant-{id}` and `sandboxes:tenant-{id}`.
- The planes link only at provisioning time. An operator provisions a sandbox, a `SessionId` is created, and that `SessionId` becomes the only link between the infrastructure record and the agent-layer record.
- After provisioning, the infrastructure plane does not join against agent-layer rows, and the agent plane does not carry infrastructure ids.
- Application developers consuming `fireline.db()` see only agent-layer state. Infrastructure state is exposed through separate admin/operator APIs such as `admin.listHosts()` and `admin.listSandboxes()`.

## Type-Enforced Boundaries

Plane separation should be enforced by published ACP schema types, not by comments or conventions.

- Rust agent-layer state uses `sacp::schema::{SessionId, RequestId, ToolCallId}` directly, not `String` and not Fireline wrapper types.
- TypeScript agent-layer state uses `@agentclientprotocol/sdk` identifier types generated from the canonical ACP schema at `https://github.com/agentclientprotocol/typescript-sdk/blob/main/schema/schema.json`, not plain `string`.
- Every struct field, function signature, and projection that represents an ACP identity must type as the canonical ACP SDK type.
- If deserialization produces `String` at the wire boundary, the immediate next step is to wrap into the canonical ACP type. Everything downstream of that boundary stays typed.
- Serialization back to JSON uses the ACP type's serde or TypeScript serializer behavior; Fireline does not invent custom stringification for ACP identities.

This turns the plane boundary into compile-time enforcement. A developer cannot accidentally pass `host_key` to a function expecting `SessionId` if those types are distinct in code.

## 1. Canonical ACP Identity Set

The relevant ACP identities already exist:

- `SessionId`: canonical ACP session identity. Source: ACP session setup spec.
  https://agentclientprotocol.com/protocol/session-setup#session-id
- `RequestId`: canonical JSON-RPC request identity. In the ACP TypeScript schema it is `null | number | string`.
  https://github.com/agentclientprotocol/typescript-sdk/blob/0d9436a12d8f3054b5dd0b2dd387dc1a8d880edd/src/schema/types.gen.ts#L3728
- `ToolCallId`: canonical tool invocation identity within a session.
  https://github.com/agentclientprotocol/typescript-sdk/blob/0d9436a12d8f3054b5dd0b2dd387dc1a8d880edd/src/schema/types.gen.ts#L5075

Fireline should standardize on two reference shapes:

`PromptRequestRef = { session_id: SessionId, request_id: RequestId }`

`ToolInvocationRef = { session_id: SessionId, tool_call_id: ToolCallId }`

These are not new identities. They are structured references composed only from canonical ACP identifiers.
Anything that cannot be expressed in those terms is either:

- transport-local bookkeeping, or
- real Fireline infrastructure identity, which belongs only in the infrastructure plane.

### W3C Trace Context for session lineage

ACP already defines the correct propagation seam.

The ACP `_meta` field is typed as `{ [key: string]: unknown }` and the protocol docs explicitly reserve these root-level keys for W3C Trace Context:

- `traceparent`
- `tracestate`
- `baggage`

Sources:

- ACP extensibility docs: https://agentclientprotocol.com/protocol/extensibility#the-_meta-field
- ACP meta-propagation RFD: https://agentclientprotocol.com/rfds/meta-propagation#implementation-details

The meta-propagation RFD states:

> The following root-level keys in `_meta` SHOULD be reserved for W3C trace context to guarantee interop with existing MCP implementations and OpenTelemetry tooling.

That means Fireline does not need a bespoke lineage schema for cross-session causality.

The trace tree already gives us:

- `trace-id`: all causally related work across sessions, agents, and hosts
- `parent-span-id`: the causal predecessor
- `span-id`: identity of the current operation
- `baggage`: propagated context data when needed

Fireline should therefore treat lineage as OpenTelemetry span structure, not Fireline state rows.

## 2. Synthetic Identifier Audit

### 2.1 Remove: synthetic lineage and approval ids

| Synthetic id | Where minted | Current consumers | Why it exists | Canonical replacement | Verdict |
|---|---|---|---|---|---|
| `prompt_turn_id` / `promptTurnId` | `crates/fireline-harness/src/state_projector.rs:302-358`, `:568-574` | `state_projector.rs`; `crates/fireline-session/src/active_turn_index.rs`; `crates/fireline-tools/src/peer/lookup.rs`, `peer/mcp_server.rs`, `peer/transport.rs`; `crates/fireline-orchestration/src/child_session_edge.rs`; `packages/state/src/schema.ts:14-20, 37-41, 55-67, 70-80, 117-125`; session-turn and turn-chunk query builders | Earlier layers could not rely on the ACP prompt request id everywhere, so Fireline minted a durable per-turn surrogate and treated it as the lineage spine | `PromptRequestRef = (session_id, request_id)` | Delete |
| `trace_id` / `traceId` | `state_projector.rs:304-329`; inherited from `_meta.fireline.traceId` in `state_projector.rs:635-699`; approval escape hatch in `crates/fireline-harness/src/approval.rs:189-205, 436-446` | `approval.rs`; `active_turn_index.rs`; `peer/lookup.rs`; `peer/mcp_server.rs:217-220`; `peer/transport.rs:167-185`; `child_session_edge.rs:22-34, 53-70`; `packages/state/src/schema.ts:19, 99, 108` | Fireline needed a stable cross-session linkage token before it adopted ACP-native trace propagation | `_meta.traceparent`, `_meta.tracestate`, `_meta.baggage`, plus OTel spans | Delete |
| `parent_prompt_turn_id` / `parentPromptTurnId` | parsed from `_meta.fireline.parentPromptTurnId` in `state_projector.rs:670-699`; written into prompt/session rows in `state_projector.rs:323-329, 403-410` | `crates/fireline-session/src/lib.rs:37-53`; `session_index.rs`; `peer/lookup.rs:11-18`; `peer/mcp_server.rs:169-183`; `peer/transport.rs:171-175`; `child_session_edge.rs:26-33, 78-89`; `packages/state/src/schema.ts:20, 100, 111` | Fireline lacked standard trace propagation, so it carried a synthetic predecessor pointer in row state | no row-level replacement; causal linkage comes from `traceparent` on propagated ACP envelopes | Delete |
| approval `request_id` hash | `crates/fireline-harness/src/approval.rs:183-205`, emitted at `:303-335` | `approval.rs:207-420, 520-541`; `packages/client/src/events.ts:10-32`; `packages/client/src/agent.ts:52-64`; `packages/state/src/schema.ts:55-68` where `requestId` and `jsonrpcId` coexist | The approval gate runs before prompt projection and could not see or trust the canonical id path, so it hashed session + policy + prompt identity | ACP `RequestId` from the actual `request_permission` JSON-RPC message; optionally paired with `ToolCallId` | Delete |

### 2.2 Delete: out-of-band row keys and transport ids

| Synthetic id | Where minted | Current consumers | Clean replacement | Verdict |
|---|---|---|---|---|
| `logical_connection_id` / `logicalConnectionId` | `crates/fireline-harness/src/routes_acp.rs:70-89` | `state_projector.rs`; `crates/fireline-session/src/lib.rs:37-53`; `packages/state/src/schema.ts:4-12, 14-17, 37-47, 55-68, 70-80, 91-104, 117-125`; connection-turn queries | No replacement in the steady state. Per-session streams and OTel span attributes are sufficient. | Delete |
| `chunk_id` / `chunkId` | `state_projector.rs:544-565` | `packages/state/src/schema.ts:117-125`; `packages/state/src/collections/turn-chunks.ts` | No replacement id. Chunk ordering comes from durable-streams offsets, which are canonical stream state rather than Fireline-minted ids. | Delete |
| `edge_id` / `edgeId` | `crates/fireline-orchestration/src/child_session_edge.rs:52-96` | `child_session_edge.rs`; `packages/state/src/schema.ts:106-115` | No replacement row key. Delete the entire bespoke edge table and use W3C trace/span relationships instead. | Delete |

### 2.3 Keep: real Fireline infrastructure ids, not ACP ids

| Id | Where minted / defined | Why it stays |
|---|---|---|
| `host_key` / `runtimeKey` | minted in `src/main.rs:344-345`, carried by `crates/fireline-session/src/host_identity.rs:82-97` | ACP has no host identity. This lives in the infrastructure materialization plane only and never appears on agent-layer rows. |
| `host_id` / `runtimeId` | minted in `crates/fireline-host/src/bootstrap.rs:136-137`, carried by `host_identity.rs:82-110` | ACP has no per-process host instance id. This lives in the infrastructure materialization plane only and never appears on agent-layer rows. |
| `node_id` / `nodeId` | minted in `src/main.rs:345`, carried by `host_identity.rs:82-110` | ACP has no cluster/node identity. This lives in the infrastructure materialization plane only and never appears on agent-layer rows. |
| `provider_instance_id` / `providerInstanceId` | assigned in `src/main.rs:370`, carried by `host_identity.rs:82-110` | External provider/container identity is outside ACP. This lives in the infrastructure materialization plane only and never appears on agent-layer rows. |

### 2.4 Out of scope: generated object ids with no ACP analogue

The grep pass also surfaced ids such as `terminalId`, `blob_key`, and provider-minted sandbox/runtime ids. Those are not ACP identity substitutes, so they are outside this proposal unless they leak into agent-layer correlation.

### 2.5 Current typed leak points

These are the current places where ACP identities are still modeled as plain `String` or plain `string` and should switch to canonical SDK types.

Rust:

- `crates/fireline-harness/src/state_projector.rs:80-109`
  `PromptTurnRow.session_id`, `PromptTurnRow.request_id`, `PendingRequestRow.request_id`, and `PendingRequestRow.session_id` are still `String`/`Option<String>`.
- `crates/fireline-harness/src/approval.rs:129-133,303-347,422-455`
  `PendingApproval.request_id`, `emit_permission_request(&str, &str, ...)`, `wait_for_approval(&str, &str)`, `is_session_approved(&str)`, `approval_timeout_error(&str)`, and `PermissionEvent.session_id` / `request_id` still use plain strings.
- `crates/fireline-session/src/active_turn_index.rs:22-25,39-43,132-149,167-171`
  `ActiveTurnRecord.session_id`, `PromptTurnRecord.session_id`, and all `&str` session-id lookup signatures still use plain strings.
- `crates/fireline-session/src/session_index.rs:28-40`
  `get(&self, session_id: &str)` and `host_spec_for_session(&self, session_id: &str)` still accept plain strings for ACP session identity.
- `crates/fireline-tools/src/peer/lookup.rs:5-27`
  `ActiveTurnRecord.prompt_turn_id`, `trace_id`, `ChildSessionEdgeInput.parent_session_id`, and `child_session_id` are all plain strings today; the surviving ACP identities should type as canonical ACP types and the synthetic ones should disappear.
- `crates/fireline-tools/src/peer/transport.rs:15-18`
  `PeerCallResult.child_session_id` is an ACP `SessionId` but is typed as `String`.

TypeScript:

- `packages/client/src/events.ts:10-15`
  `appendApprovalResolved` currently takes `sessionId: string` and `requestId: string`.
- `packages/client/src/agent.ts:52-55`
  `resolvePermission(sessionId: string, requestId: string, ...)` should use ACP SDK identifier types.
- `packages/state/src/schema.ts:14-20,37-67,91-123`
  `sessionId`, `requestId`, and `toolCallId` are all declared with `z.string()` today, which infers plain `string` on the public row types. Agent-layer public row types should expose ACP SDK identifier types generated from `schema/schema.json` instead.

## 3. Synthetic-Only Structures That Should Disappear

### 3.1 `TraceCorrelationState`

Defined in `crates/fireline-harness/src/state_projector.rs:166-174`.

What exists only because of synthetic ids:

- `prompt_request_to_turn`: needed only because the durable row key is not the ACP request id.
- `session_active_turn`: needed because peer propagation currently depends on Fireline-specific synthetic lineage rather than standard trace-context passthrough.
- `chunk_seq`: ordering state keyed by a synthetic prompt-turn id.
- `turn_counter`: exists only to mint `prompt_turn_id`.

After the change:

- `turn_counter` disappears.
- `prompt_request_to_turn` disappears; the prompt row is already keyed by `(session_id, request_id)`.
- `session_active_turn` disappears.
- `chunk_seq` disappears because chunk ordering comes from durable-streams offsets rather than a Fireline-maintained counter.

### 3.2 `InheritedLineage`

Defined in `state_projector.rs:177-180`.

This structure should be deleted, not renamed or replaced.

Why:

- ACP already provides `_meta` propagation for context
- `state_projector.rs:401-410` currently writes inherited lineage onto `SessionRecord`, which denormalizes the wrong relationship onto every session row
- lineage belongs in W3C Trace Context, not Fireline state rows

Propagation should happen via `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage` passthrough, which ACP already allows. No replacement struct is needed.

### 3.3 `ActiveTurnIndex`

`crates/fireline-session/src/active_turn_index.rs` is 250 lines of state that exists almost entirely to bridge from session id to Fireline's invented prompt-turn lineage.

Today it returns:

- `prompt_turn_id`
- `trace_id`

to peer code at `crates/fireline-tools/src/peer/mcp_server.rs:210-220`.

Clean end state: tools receive `ToolCallId` directly from ACP tool execution context, prompt-side lineage uses `RequestId`, and peer handoff propagates W3C Trace Context via `_meta`.

`ActiveTurnIndex` is deleted. If ACP tool execution context does not currently expose `ToolCallId` where Fireline needs it, file an upstream issue against `agent-client-protocol-core` and block this refactor until it is resolved or a non-synthetic ACP-compliant alternative is confirmed. Fireline must not ship a synthetic surrogate as a workaround.

### 3.4 Session rows carrying lineage and infrastructure baggage

`crates/fireline-session/src/lib.rs:37-53` stores `trace_id` and `parent_prompt_turn_id` on `SessionRecord`.

That is not session identity. It is leaked call-lineage state.

`SessionRecord` should keep only:

- `session_id`
- `supports_load_session`
- `created_at`
- `updated_at`

and drop:

- `host_key`
- `host_id`
- `node_id`
- `logical_connection_id`
- `trace_id`
- `parent_prompt_turn_id`

Rationale: a session has no host in the agent plane. It has a stream. Any "which host provisioned this session?" query belongs to the infrastructure plane, not to `SessionRecord`.

## 4. Proposed Canonical Replacements

### 4.1 Prompt lifecycle

Current: `prompt_turn_id = "{host_id}:{logical_connection_id}:{counter}"`

Proposed: `prompt_request_ref = { session_id, request_id }`

Implications:

- `prompt_turn` should be renamed to `prompt_request`, or at minimum re-keyed by canonical request identity
- inside `state/session/{session_id}`, the canonical prompt row key is simply `request_id`
- `pending_request.prompt_turn_id` disappears
- no infrastructure ids appear on prompt rows

### 4.2 Approval lifecycle

Current:

- synthetic `Permission.requestId`
- canonical `Permission.jsonrpcId`
- optional `Permission.toolCallId`

That schema is already admitting the problem: it has both the fake id and the real one.

Proposed: `PermissionRef = { session_id, request_id }` and `PermissionRow = { session_id, request_id, tool_call_id?, state, outcome, ... }`, where `request_id` is the canonical JSON-RPC permission request id.

The approval gate should emit and wait on the actual JSON-RPC `request_permission` id, not a SHA256 of prompt text.

`packages/client/src/events.ts:10-32` and `packages/client/src/agent.ts:52-64` then become correct as written: `requestId` means ACP `RequestId`, not "Fireline approval hash".

### 4.3 W3C Trace Context for session lineage

ACP's `_meta` field already reserves W3C Trace Context keys for exactly this problem:

- `traceparent`
- `tracestate`
- `baggage`

Fireline should adopt that directly.

Instrumentation points:

- `session/new` -> start a span, using incoming `_meta.traceparent` when present
- `session/prompt` -> start a span under the session span
- tool call -> start a span under the prompt span
- peer call outbound -> start a span and inject `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage` into the outgoing ACP request
- peer call inbound -> extract `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage`, then start the child span
- `approval_gate` emit -> span with approval attributes such as policy id, decision mode, and timeout
- `approval_resolved` -> event or span completion on the same trace

The trace tree is the lineage. Fireline ships no bespoke lineage schema. If an app wants to answer "what sessions did session S1 invoke?", that is an OpenTelemetry query over spans, not a Fireline-specific edge-table query. Fireline may materialize a local view over spans for convenience, but the trace remains the source of truth.

### 4.4 Chunks

Current chunk identity:

- synthetic `chunk_id`
- synthetic foreign key `prompt_turn_id`

Proposed: `ChunkRow { session_id, request_id, tool_call_id?, type, content, created_at }`, where `tool_call_id` is present only when the chunk is attributable to a tool call.

There is no synthetic `seq` and no synthetic `chunk_id`. Chunk ordering comes from the durable-streams offset. If a client needs ordering surfaced explicitly, it should use the stream offset rather than a Fireline-minted ordinal.

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

Prefer renaming collection/entity type from `promptTurns` / `prompt_turn` to `promptRequests` / `prompt_request`. There is no ACP "turn" identifier to preserve. Prompt rows stay flat: no `traceId`, predecessor ids, caller metadata, or infrastructure ids. Correlation comes from the ACP request envelope's `_meta.traceparent` when needed.

### 5.2 Permission rows

Current:

- `requestId`
- `jsonrpcId`
- `promptTurnId`
- `toolCallId?`

Proposed: `requestId` becomes the canonical JSON-RPC id, `jsonrpcId` and `promptTurnId` are removed, and `toolCallId?` remains. Permission rows stay flat and carry no infrastructure ids; `traceparent` and `tracestate` join approval work to the same trace without Fireline lineage fields.

### 5.3 Session rows

Current:

- `traceId?`
- `parentPromptTurnId?`
- `logicalConnectionId`

Proposed: keep `sessionId`, `supportsLoadSession`, `createdAt`, and `updatedAt`; remove `traceId`, `parentPromptTurnId`, `runtimeKey`, `runtimeId`, `nodeId`, and `logicalConnectionId`; do not add caller, invoker, host, sandbox, or provider fields.

### 5.4 Child-session edge rows

Delete the `child_session_edge` entity entirely.

If an application needs cross-session lineage queries, it should query OpenTelemetry spans or a trace-derived materialized view, not Fireline state rows.

### 5.5 Chunks

Current:

- `chunkId`
- `promptTurnId`

Proposed: `sessionId`, `requestId`, `toolCallId?`, `type`, `content`, `createdAt`. Ordering comes from the durable-streams offset, not a Fireline-managed `seq`.

## 6. Durable-Streams Sessions

Durable Streams does not appear to provide a special protocol-level "ACP session" primitive. The integration doc describes a durable sessions pattern:

- one JSON-mode stream per session
- session structure lives in the event payload
- the stream is replayed and tailed via SSE

Source:
https://thesampaton.github.io/durable-streams-rust-server/integration/sessions.html

So the clean mapping is architectural rather than built into the DS wire format: `ACP SessionId 1:1 Fireline session stream name`.

Agent-layer state lives in per-session streams such as `state/session/{session_id}`. These streams carry only agent-layer state.

Infrastructure-layer state lives in separate streams such as `hosts:tenant-{id}` and `sandboxes:tenant-{id}`.

What collapses if Fireline adopts that mapping:

- approval replay no longer scans a tenant-wide stream and filters by `sessionId`; it reads that session's stream only
- prompt/chunk/session lifecycle rows no longer need `sessionId` as a secondary filter over a shared stream; it is implicit in the stream
- `SessionIndex` can shrink from "global shared-stream materializer" to "per-session materializer + optional cross-session registry"
- durable subscribers become naturally session-scoped

What does not collapse: host discovery remains tenant-wide or host-wide; lineage still lives in W3C Trace Context and the trace backend rather than Fireline session state; infrastructure ids still exist because DS sessions do not replace host identity.

There is no session row field that references a host, and no host row is part of the agent-layer stream. The only shared token at provisioning time is `SessionId`.

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
3. keep lineage out of the core state model entirely; it is carried by `_meta` trace propagation and verified separately from session/prompt state
4. keep `HarnessSuspendReleasedOnlyByMatchingApproval`; it already states the right invariant once `req` is canonical

This is good news: the TLA model mostly needs alignment and naming cleanup, not conceptual redesign.

## 8. Downstream Dependencies

### 8.1 `DurableSubscriber`

`docs/proposals/durable-subscriber.md:9-20, 28-35` generalizes the approval gate. That proposal should not bake in the approval gate's current synthetic `request_id` hash.

`DurableSubscriber` should standardize on:

- canonical request ids
- canonical session ids
- optional tool call ids
- `sacp::schema::{SessionId, RequestId, ToolCallId}` in Rust trait signatures rather than generic `String` keys

Otherwise Fireline will generalize the wrong abstraction.

### 8.2 Platform SDK

`docs/proposals/platform-sdk-api-design.md:13-15, 27-45, 105-117` already wants `fireline.db()` as the global state entry point and `agent.resolvePermission(sessionId, requestId, ...)`. That API becomes unambiguous only after `requestId` means ACP `RequestId`, not Fireline's hash.

## 9. Implementation Plan

### Phase 0: ACP prerequisite

If ACP tool execution context does not expose `ToolCallId` where Fireline needs it, file an upstream issue against `agent-client-protocol-core` and do not proceed with this refactor until that is resolved or a non-synthetic ACP-compliant alternative is confirmed.

### Phase 1: introduce canonical ref types

Add shared Rust and TypeScript types:

- `PromptRequestRef`
- `ToolInvocationRef`
- trace-context extraction/injection helpers

### Phase 1.5: replace plain-string ACP identifiers

Replace all plain-string ACP identifier fields on agent-layer structures with canonical SDK types:

- Rust: `sacp::schema::{SessionId, RequestId, ToolCallId}`
- TypeScript: `@agentclientprotocol/sdk` identifier types

This mechanical pass surfaces every remaining place where Fireline treated an ACP identity as an untyped string.

### Phase 2: dual-write canonical rows

Do not mutate old entity shapes in place while readers still replay historical streams.

Prefer one of:

- new entity types, e.g. `prompt_request`, `permission_v2`, or
- backward-compatible readers that accept both old and new row shapes

The safer cut is new entity types. Historical synthetic events already in append-only streams cannot be rewritten.

### Phase 3: approval gate first

Change `crates/fireline-harness/src/approval.rs` to emit and wait on the real JSON-RPC permission request id.

Why first: it is the smallest isolated win, it unblocks `DurableSubscriber`, and it makes the Platform SDK's `resolvePermission(sessionId, requestId)` semantically correct.

### Phase 4: prompt/chunk/session projection

Change `StateProjector` so prompt lifecycle rows are keyed by canonical prompt request identity.

This removes `next_prompt_turn_id()`, `turn_counter`, `prompt_request_to_turn`, `logical_connection_id` from agent-layer rows, `chunk_id`, and `chunk_seq`.

and moves session rows to the agent plane only, with no host or provider fields.

### Phase 5: trace propagation and OTel

Update:

- `crates/fireline-tools/src/peer/lookup.rs`
- `peer/mcp_server.rs`
- `peer/transport.rs`

to emit OpenTelemetry spans and propagate `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage` on outbound and inbound peer calls.

### Phase 6: remove synthetic-only structures

Delete or collapse:

- `TraceCorrelationState` fields that only exist for synthetic ids
- `InheritedLineage`
- `ActiveTurnIndex`
- `trace_id` and `parent_prompt_turn_id` from `SessionRecord`
- the entire `child_session_edge` table and its schema
- any infrastructure ids currently projected onto agent-layer rows

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
- peer propagation tests around `_meta.traceparent` and OTel span parenting

Backward-compatibility concerns:

- existing streams already contain synthetic `prompt_turn`, `permission`, `child_session_edge`, and `chunk` rows
- append-only history means we cannot rewrite those rows
- readers must either support both shapes or switch to versioned entity types

Recommendation: dual-read for one migration window, dual-write only if necessary, then stop writing synthetic rows and delete their readers.

## 11. Final Recommendation

Fireline should draw a hard boundary:

- ACP identifiers are the only canonical identifiers for session, request, and tool-call lineage.
- agent-layer state uses only ACP-schema identifiers and W3C Trace Context via `_meta`.
- infrastructure-layer state is isolated to its own streams and is never joined against agent-layer state.
- W3C Trace Context in `_meta` carries cross-session causality.
- Derived storage keys stay derived and stop leaking into the public domain model.

Concretely:

- delete `prompt_turn_id`
- delete `trace_id`
- delete `parent_prompt_turn_id`
- delete the approval gate's hashed `request_id`
- delete `logical_connection_id`
- delete `chunk_id`
- delete `chunk_seq`
- delete `child_session_edge` and `edge_id`
- keep `SessionId`, `RequestId`, and `ToolCallId` in the agent plane
- keep `host_key`, `runtimeId`, `node_id`, and `provider_instance_id` only in the infrastructure plane

Cross-session causality survives as W3C Trace Context and OpenTelemetry spans. It does not belong on session rows, prompt rows, chunk rows, permission rows, or a bespoke Fireline edge table.

## 12. What Doesn't Change

This proposal does not collapse Fireline's core state model.

These remain: `SessionId`, `RequestId`, and `ToolCallId` as the canonical ACP identifiers; `SessionRecord`, but without lineage baggage; `PermissionRow`, keyed by canonical request id; and host/runtime infrastructure ids such as `host_key`, `runtimeId`, `node_id`, and `provider_instance_id`, but only in the infrastructure plane.

The change is specifically that lineage stops being modeled as Fireline row state and starts being modeled as ACP-native W3C Trace Context.

## 13. OpenTelemetry Integration

Fireline should emit spans via the Rust `opentelemetry` crate and propagate W3C Trace Context through ACP `_meta`.

Requirements:

- outbound peer calls must inject `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage`
- inbound peer calls must extract those fields and join the existing trace
- Fireline-specific details such as `approval.policy_id`, `approval.mode`, or `host_key` belong in OTel span attributes, not in bespoke lineage ids
- the trace backend is configurable via OTLP export to systems such as Jaeger, Tempo, Honeycomb, or Datadog; backend choice is not Fireline's concern

## 14. Validation Checklist

- [x] No entity in the proposal schema carries a synthetic id as primary or correlation key
- [x] No synthetic lineage fields (`parent_*`, `trace_id`, parallel graph ids) remain on any agent/session/prompt/tool row
- [x] All graph stitching is done via ACP-schema identifiers or W3C Trace Context via `_meta`
- [x] No bespoke edge/lineage table exists for agent/session/prompt/tool correlation; the trace backend handles lineage
- [x] Every infrastructure id retained is called out with a justification: lives in the infrastructure materialization plane only and is never joined against agent-layer state
- [x] Every derived storage key is documented as a concatenation of canonical ACP ids, not a new identity
- [x] Agent-layer rows (`session`, `prompt_request`, `chunk`, `permission`, `tool_call`) contain zero Fireline-minted identifiers
- [x] Agent-layer Rust structs type ACP identifier fields as `sacp::schema::{SessionId, RequestId, ToolCallId}`, not `String`
- [x] Agent-layer TypeScript schemas and public row types type ACP identifier fields as `@agentclientprotocol/sdk` types, not plain `string`
- [x] No `String -> SessionId` conversion happens outside a boundary layer such as wire or JSON deserialization
- [x] Infrastructure-layer types (`host_key`, `host_id`, `node_id`, `provider_instance_id`) remain plain strings or infrastructure newtypes and are explicitly not ACP types
- [x] Infrastructure-layer rows (`host`, `sandbox`, `provider`, `node`) live in separate streams and are not joined against agent-layer rows
- [x] No synthetic ordinal, counter, or monotonic id is used as a correlation key on agent-layer rows; chunk ordering comes from durable-streams offset
- [x] `logical_connection_id` does not appear anywhere in the proposed steady-state design
- [x] No temporary bridge or optional hedge remains; every identifier is either in or out
- [x] Grep audit: searching the proposal for `trace_id`, `prompt_turn`, `parent_`, `caller_`, `child_`, `correlation`, and `edge_id` only finds acceptance text, historical-audit/removal text, or migration/removal notes; none remain in the proposed steady-state design

If Fireline does not make this cut before `DurableSubscriber`, it will freeze old accidental identities into the next layer of generic abstractions.
