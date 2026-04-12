# State Projector Surface Audit

This audit reviews the projection surface in `crates/fireline-harness/src/state_projector.rs` before canonical-identifiers Phase 3. The goal is to separate rows that earn their existence as durable read models from rows and helpers that are only carrying synthetic identity, trace glue, or legacy connection concepts.

This is a planning document, not an implementation patch. Architect review should decide whether the deletion candidates below land inside Phase 3 or are split into a follow-on Phase 3.5.

## 1. Inventory

`state_projector.rs` currently defines these projection-facing or projection-supporting types:

- `ConnectionRow`
- `PromptTurnRow`
- `PendingRequestRow`
- `HostInstanceRow`
- `ChunkRow`
- `StateHeaders`
- `StateEnvelope`
- `TraceCorrelationState`
- `InheritedLineage`
- `TraceEndpoint`
- `PendingRequestDirection`
- `ConnectionState`
- `PromptTurnState`
- `PendingRequestState`
- `HostInstanceState`
- `ChunkType`

## 2. Classification rules

- `AGGREGATE`: synthesizes durable view state from multiple ACP events and still earns a first-class row after canonical rekeying.
- `DENORM`: caches a wire shape or event fragment for read performance, but should lean on ACP schema types where possible.
- `WASTE`: synthetic wrapper, internal helper, or unused/infra-only surface that should be deleted or moved off the agent plane.

Current readback evidence says the real consumer pressure is concentrated in:

- `PromptTurnRow`: read by `packages/state/src/collections/*`, `examples/*`, and Flamecast session views.
- `ChunkRow`: heavily read by examples and Flamecast transcript assembly.
- `PendingRequestRow`: little to no direct app readback.
- `ConnectionRow` and `HostInstanceRow`: effectively no app-level readback; they mainly survive because the schema still exposes them.

## 3. Consumer readback summary

- `packages/client/src/db.ts` is just a pass-through; it does not justify any field by itself.
- `packages/state/src/collections/session-turns.ts`, `active-turns.ts`, `queued-turns.ts`, `connection-turns.ts`, and `turn-chunks.ts` currently encode the synthetic keys: `promptTurnId`, `logicalConnectionId`, `chunkId`, `seq`.
- The heaviest real consumer is `examples/flamecast-client/server.ts`, which reads `turn.text`, `turn.stopReason`, `turn.promptTurnId`, `chunk.type`, `chunk.content`, and `chunk.seq`, then reconstructs a transcript.
- Other examples mostly read `sessionId`, prompt-turn `state`, `text`, permission `requestId`, and chunk `content`.

## 4. Deletion proposals

- Delete from the agent-plane projection: `ConnectionRow`, `ConnectionState`, `HostInstanceRow`, `HostInstanceState`, `TraceCorrelationState`, `InheritedLineage`, `TraceEndpoint`, `StateHeaders`, `StateEnvelope`.
- Delete or move behind a diagnostics-only surface: `PendingRequestRow`, `PendingRequestState`, `PendingRequestDirection`.
- Replace `ChunkType` plus `content: String` with a typed `SessionUpdate`-backed row or live view; do not carry the current lossy flattening forward as-is.
- Rename and rekey `PromptTurnRow` into a canonical request-scoped row keyed by `(session_id, request_id)`; remove synthetic identifiers even if the row itself survives.

## 5. Row and enum review

### ConnectionRow
| current shape | classification | proposed retypings | proposed deletions | consumer-readback evidence |
| --- | --- | --- | --- | --- |
| `logical_connection_id`, `state`, `latest_session_id`, `last_error`, `queue_paused`, timestamps | `WASTE` on the agent plane. It exists to model a synthetic conductor connection, not an ACP identity. | `latest_session_id` should be `Option<SessionId>` if this row survives anywhere, but the better move is to keep session identity on `SessionRecord` and remove the row from `fireline.db()`. | Delete `logical_connection_id`; likely delete the whole row from the public projection. If connection health remains useful, move it to an infra/admin stream keyed by sandbox or host id. | Direct example reads are effectively zero. Only `packages/state/src/collections/connection-turns.ts` depends on `logicalConnectionId`, and that collection is synthetic too. |

### PromptTurnRow
| current shape | classification | proposed retypings | proposed deletions | consumer-readback evidence |
| --- | --- | --- | --- | --- |
| `prompt_turn_id`, `logical_connection_id`, `session_id`, `request_id`, `trace_id`, `parent_prompt_turn_id`, `text`, `state`, `position`, `stop_reason`, timestamps | `AGGREGATE`, but overspecified. It combines request, response, and streamed updates into one request-scoped lifecycle row. | Keep `session_id: SessionId` and `request_id: RequestId`. Retype `stop_reason: Option<sacp::schema::StopReason>`. `text: Option<String>` should either become `Option<Vec<sacp::schema::ContentBlock>>` if the row owns prompt content, or be dropped and derived as a preview outside the canonical row. | Delete `prompt_turn_id`, `logical_connection_id`, `trace_id`, `parent_prompt_turn_id`, and likely `position`. Rename the row to `PromptRequestRow` and key it by `(session_id, request_id)` instead of a synthetic turn id. | Widely read: `packages/state` turn collections, `examples/crash-proof-agent`, `examples/multi-agent-team`, `examples/background-task`, `examples/live-monitoring`, and Flamecast all read `sessionId`, `state`, `text`, and/or `stopReason`. |

### PendingRequestRow
| current shape | classification | proposed retypings | proposed deletions | consumer-readback evidence |
| --- | --- | --- | --- | --- |
| `request_id`, `logical_connection_id`, `session_id`, `prompt_turn_id`, `method`, `direction`, `state`, timestamps | `WASTE` in the current public projection. It is mostly a trace/debug cache of outbound JSON-RPC requests. | Keep `request_id: RequestId` and `session_id: Option<SessionId>` only if the row survives as an internal debug surface. `method: String` has no good ACP schema enum today; if retained, use a tiny Fireline enum for the few methods actually projected. | Delete `logical_connection_id` and `prompt_turn_id`. Strong candidate to delete the row entirely from `fireline.db()` and keep only permission-specific state as first-class rows. | No meaningful app-level reads showed up in `packages/client` or the examples. The public schema exposes it, but the examples do not build UI on top of it. |

### HostInstanceRow
| current shape | classification | proposed retypings | proposed deletions | consumer-readback evidence |
| --- | --- | --- | --- | --- |
| `instance_id`, `runtimeName`, `status`, timestamps | `DENORM` for infra/admin, `WASTE` for the agent plane. It models host lifecycle, not ACP session identity. | No ACP schema type applies. If retained, it belongs in an admin or deployment stream keyed by host/sandbox identity, not the agent-plane state projection. | Remove the row from `fireline.db()` and from Phase 3 canonical-id scope; keep it only in infra-plane APIs. | I did not find meaningful reads in `packages/client` or the examples. It is exposed in schema but not driving user-facing flows. |

### ChunkRow
| current shape | classification | proposed retypings | proposed deletions | consumer-readback evidence |
| --- | --- | --- | --- | --- |
| `chunk_id`, `session_id`, `prompt_turn_id`, `logical_connection_id`, `type`, `content`, `seq`, `created_at` | `DENORM`, but the current denorm is lossy and synthetic-heavy. Each row is basically a flattened `session/update`. | Keep `session_id: SessionId`. Replace `type + content` with a typed ACP payload, ideally `sacp::schema::SessionUpdate` or a small typed wrapper around it. If a flattened content preview is still needed, make it explicitly derived. | Delete `chunk_id`, `prompt_turn_id`, `logical_connection_id`, and `seq` in favor of canonical request scope plus durable-stream offset ordering. This is the main candidate for a Phase 3.5 split because consumers currently rely on those fields. | High readback pressure: `packages/state/src/collections/turn-chunks.ts`, `examples/multi-agent-team`, `examples/live-monitoring`, and Flamecast transcript builders all read `type`, `content`, `seq`, and `promptTurnId`. |

### StateHeaders
| current shape | classification | proposed retypings | proposed deletions | consumer-readback evidence |
| --- | --- | --- | --- | --- |
| `operation` wrapper used in `StateEnvelope` | `WASTE` as a surface type. It is an envelope helper, not part of the domain model. | None. | Delete from the audited surface inventory once envelope plumbing is hidden behind stream-state infrastructure. | No consumer reads this directly. |

### StateEnvelope
| current shape | classification | proposed retypings | proposed deletions | consumer-readback evidence |
| --- | --- | --- | --- | --- |
| `type`, `key`, `headers`, `value` wrapper used by `state_change()` | `WASTE` as a projector surface type. It is transport glue. | None. | Delete from the proposal surface; keep only as an internal serialization helper if the state transport still needs it. | No consumer reads this directly; consumers see materialized collections, not envelopes. |

### TraceCorrelationState
| current shape | classification | proposed retypings | proposed deletions | consumer-readback evidence |
| --- | --- | --- | --- | --- |
| `pending_initialize`, `prompt_request_to_turn`, `prompt_turns`, `pending_requests`, `session_active_turn`, `chunk_seq`, `turn_counter` | `WASTE`. This is the synthetic-ID bookkeeping that canonical-identifiers Phase 3 is supposed to delete. | `pending_initialize` can stay request-keyed by canonical `RequestId` if still needed. Everything else should move to canonical `(session_id, request_id)` indexing or disappear. | Delete `prompt_request_to_turn`, `session_active_turn`, `chunk_seq`, and `turn_counter`. Keep only the minimal canonical correlation state required during request/response projection. | Internal-only helper; no public readback. |

### InheritedLineage
| current shape | classification | proposed retypings | proposed deletions | consumer-readback evidence |
| --- | --- | --- | --- | --- |
| `trace_id`, `parent_prompt_turn_id` parsed from `_meta.fireline.*` | `WASTE`. It carries bespoke lineage fields that canonical-identifiers explicitly replaces. | No ACP type fits because the proposal direction is W3C trace context, not typed Fireline lineage ids. | Delete the helper and stop parsing `_meta.fireline.traceId` / `_meta.fireline.parentPromptTurnId`; Phase 5 should use `_meta.traceparent`, `_meta.tracestate`, and `baggage` instead. | Internal-only helper; no public readback. |

### TraceEndpoint
| current shape | classification | proposed retypings | proposed deletions | consumer-readback evidence |
| --- | --- | --- | --- | --- |
| `Client`, `Agent`, `Proxy(usize)`, `Unknown` helper for `from`/`to` trace labels | `WASTE` as a durable-model concern. It is only request-routing glue for the current conductor topology. | None. | Delete from the projection surface once Phase 3 stops keying state off conductor/proxy hops. | Internal-only helper; no public readback. |

### PendingRequestDirection
| current shape | classification | proposed retypings | proposed deletions | consumer-readback evidence |
| --- | --- | --- | --- | --- |
| `ClientToAgent`, `AgentToClient` | `WASTE` if `PendingRequestRow` is deleted. | None worth carrying into ACP types; this is local debug metadata. | Delete with `PendingRequestRow`, or keep only in a diagnostics stream that is not part of `fireline.db()`. | No meaningful consumer readback. |

### ConnectionState
| current shape | classification | proposed retypings | proposed deletions | consumer-readback evidence |
| --- | --- | --- | --- | --- |
| `Created`, `Attached`, `Broken`, `Closed` | `WASTE` with `ConnectionRow`. | None. | Delete with `ConnectionRow`, or move into an infra/admin plane if connection health still matters operationally. | No direct app-level reads. |

### PromptTurnState
| current shape | classification | proposed retypings | proposed deletions | consumer-readback evidence |
| --- | --- | --- | --- | --- |
| `Queued`, `Active`, `Completed`, `CancelRequested`, `Cancelled`, `Broken`, `TimedOut` | `AGGREGATE`, but currently overdeclared. The projector only writes `Active`, `Completed`, and `Broken` today. | No ACP schema enum directly replaces this row-lifecycle state. Keep it as a Fireline aggregate enum if the prompt-request row survives. | Trim unused variants unless Phase 3 also lands the events that can actually materialize them. | Consumers do read `state`, especially in examples and `packages/state` collections, but they only appear to rely on `active` and `completed` today. |

### PendingRequestState
| current shape | classification | proposed retypings | proposed deletions | consumer-readback evidence |
| --- | --- | --- | --- | --- |
| `Pending`, `Resolved`, `Orphaned` | `DENORM` only if `PendingRequestRow` survives. | None from ACP schema. | Delete with `PendingRequestRow`, or at minimum trim `Orphaned` until the projector can actually emit it. | No meaningful consumer readback. |

### HostInstanceState
| current shape | classification | proposed retypings | proposed deletions | consumer-readback evidence |
| --- | --- | --- | --- | --- |
| `Running`, `Paused`, `Stopped` | `DENORM` for infra/admin, `WASTE` for agent plane. | No ACP schema type applies. | Move with `HostInstanceRow` to infra/admin surfaces; remove from `fireline.db()`. | No meaningful consumer readback. |

### ChunkType
| current shape | classification | proposed retypings | proposed deletions | consumer-readback evidence |
| --- | --- | --- | --- | --- |
| `Text`, `ToolCall`, `Thinking`, `ToolResult`, `Error`, `Stop` | `WASTE` as a parallel enum. It re-encodes a subset of `SessionUpdate` and loses detail. | Replace the enum with typed ACP payloads: `SessionUpdate`, `ContentBlock`, `ToolCall`, `ToolCallUpdate`, and `StopReason` are the right underlying types. | Delete once `ChunkRow` stops flattening updates into `type + content: String`. | Heavy current readback via examples and Flamecast, but only because the typed alternative does not exist yet. |

## 6. Recommended Phase 3 boundary

- Safe-to-delete in Phase 3: `TraceCorrelationState`, `InheritedLineage`, `TraceEndpoint`, `StateHeaders`, `StateEnvelope`, and likely `ConnectionRow` plus `ConnectionState`.
- Safe-to-move out of the agent plane in Phase 3: `HostInstanceRow` plus `HostInstanceState`.
- High-value keeper to rekey in Phase 3: `PromptTurnRow`, but as a trimmed canonical request row.
- Highest-risk redesign: `ChunkRow`. It has the strongest consumer coupling, so Architect review should decide whether typed `SessionUpdate` replacement lands in Phase 3 or in a dedicated Phase 3.5 with coordinated TS/query updates.
- Low-value surface to delete unless a diagnostics requirement appears: `PendingRequestRow` plus `PendingRequestDirection` and `PendingRequestState`.
