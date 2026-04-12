# State Projector Audit Review (Architect Synthesis)

> Date: 2026-04-12
> Reviewer: Architect (Opus 3)
> Reviews: [state-projector-surface-audit.md](../proposals/state-projector-surface-audit.md)
> Updates: [acp-canonical-identifiers-execution.md §Phase 3](../proposals/acp-canonical-identifiers-execution.md#phase-3-stateprojector-canonical-rekeying)
> Blocks cleared: canonical-ids Phase 3 dispatch

## TL;DR

The audit is well-grounded: classification matches consumer-readback evidence, and most proposals line up with the governing [acp-canonical-identifiers.md](../proposals/acp-canonical-identifiers.md).

**Decision: FOLD most of the audit's additions into Phase 3; SPLIT the chunk-payload redesign into a new Phase 3.5.**

- Phase 3 core (as already written in the execution plan) is unchanged.
- Phase 3 gains six low-coupling additions from the audit (ConnectionRow/State delete, HostInstanceRow move, PendingRequestRow delete-or-diagnose, TraceEndpoint delete, `stop_reason` retype, `PromptTurnState` variant trim).
- Phase 3.5 (new) covers the `ChunkType + content: String` → typed `sacp::schema::SessionUpdate` redesign. Splitting keeps Phase 3's revert granularity tight and isolates the high-consumer-coupling semantic change for independent CI pressure.
- Two audit proposals are REJECTED (`StateHeaders`, `StateEnvelope` — keep as private module-internal helpers; not consumer-visible, not in scope).

Phase 3 LOC estimate: revised ~500 → ~850 (still single-phase-revertable). Phase 3.5 LOC: ~250-400 depending on coordinated TS and Flamecast-shim work.

**Additionally (§8):** `crates/fireline-orchestration/src/child_session_edge.rs` pulled up from Phase 5 into Phase 3. Consumer grep confirms zero agent-plane readers; pure tech debt; plane-muddying (mixes infra + agent identifiers in one row) + synthetic SHA256 edge_id. Phase 5 scope reduces to `ActiveTurnIndex`-only deletion.

No invariant conflicts. Phase 6 TS cascade flagged at §5.

---

## 1. Per-proposal verdict

Legend: `A` = accept as written, `A*` = accept with modification, `M` = modify / scope change, `R` = reject, `D` = defer (wrong phase).

| # | Audit target | Audit proposal | Verdict | Placement | Architect note |
|---|---|---|---|---|---|
| 1 | `ConnectionRow` + `ConnectionState` | Delete from agent plane | **A** | Phase 3 | Matches canonical-ids §2.2 delete list for `logical_connection_id`. No app-level consumer. Coordinate with Phase 6 (removes `createConnectionTurnsCollection`). |
| 2 | `PromptTurnRow` — rekey | Rename to `PromptRequestRow`, key by `(SessionId, RequestId)`, drop `prompt_turn_id` / `logical_connection_id` / `trace_id` / `parent_prompt_turn_id` / `position` | **A** | Phase 3 (already in exec plan) | Already in §Phase 3.B. Audit confirms; no scope change. |
| 3 | `PromptTurnRow` — `stop_reason` | Retype `Option<String>` → `Option<sacp::schema::StopReason>` | **A** | Phase 3 | Low-risk additive retyping. Consumers currently read `stopReason` as string; `StopReason` serializes as a string enum, so wire shape preserved. |
| 4 | `PromptTurnRow` — `text` field | Either `Option<Vec<ContentBlock>>` or derived-preview | **A\*** | Phase 3 | **Keep as `Option<String>` derived preview.** Making it `Vec<ContentBlock>` would duplicate chunk content and create a two-truths hazard. Add a `// derived preview` comment at the field. |
| 5 | `PromptTurnState` | Trim unused variants | **A\*** | Phase 3 | Trim `Queued` / `CancelRequested` / `Cancelled` / `TimedOut` unless the projector actually materializes them. Verify against TLA+ `SessionEventKind` set before deletion; if any invariant expects a variant, keep that variant. |
| 6 | `PendingRequestRow` + `PendingRequestDirection` + `PendingRequestState` | Delete or move to diagnostics stream | **A\*** | Phase 3 | **Delete from agent-plane projection.** No consumer reads. If operational diagnostics become necessary later, reintroduce in an infra/admin stream under a separate proposal — don't pre-build a diagnostics surface speculatively. |
| 7 | `HostInstanceRow` + `HostInstanceState` | Move out of `fireline.db()` to infra/admin | **A** | Phase 3 | Matches plane-separation. Move to `hosts:tenant-{id}` stream surface already planned in canonical-ids §Plane Separation. Phase 7 (plane enforcement) already assumes this is done — pulling it into Phase 3 accelerates the invariant. |
| 8 | `TraceCorrelationState` | Delete synthetic-ID bookkeeping (`turn_counter`, `prompt_request_to_turn`, `session_active_turn`, `chunk_seq`) | **A** | Phase 3 (already in exec plan) | Already in §Phase 3.B. Keep `pending_initialize` and `pending_requests` (rekeyed to canonical `RequestId`) for response correlation during projection. |
| 9 | `InheritedLineage` | Delete helper; stop parsing `_meta.fireline.*` | **A** | Phase 3 (already in exec plan) | Already in §Phase 3.B. Note: between Phase 3 landing and Phase 4 landing, trace lineage is lost. This is expected per the execution plan's Phase 4→5 ordering. |
| 10 | `TraceEndpoint` | Delete | **A** | Phase 3 | Internal routing label, no durable-model concern. Confirm via grep that it's not written to a state stream (audit says internal-only; verify in the dispatch). |
| 11 | `StateHeaders` | Delete from surface inventory | **R** | — | Keep as a private module-internal helper. Not consumer-visible; not part of the audited public surface. Deleting requires replacing `state_change()` plumbing — out of Phase 3 scope. Flag as a future refactor if ever justified. |
| 12 | `StateEnvelope` | Delete from surface | **R** | — | Same as #11. Internal serialization helper. Consumers see materialized collections, not envelopes. Keep as private. |
| 13 | `ChunkRow` — rekey by canonical | Drop `chunk_id` / `prompt_turn_id` / `logical_connection_id` / `seq`; key by `(SessionId, RequestId[, ToolCallId?])`; ordering from durable-stream offset | **A** | Phase 3 (already in exec plan) | Already in §Phase 3.B. Phase 3 dual-writes `chunk_v2` under the canonical key; old `chunk` readers survive until Phase 8. |
| 14 | `ChunkRow` — `type + content: String` redesign | Replace with typed `sacp::schema::SessionUpdate` | **A\*** | **Phase 3.5** (SPLIT) | **High-consumer-coupling semantic redesign.** Flamecast, `examples/multi-agent-team`, `examples/live-monitoring`, and `turn-chunks.ts` all pattern-match `type` and read `content` as string. Folding into Phase 3 forces one phase to both rekey AND redesign the payload — two orthogonal axes. Splitting keeps the rekey independently revertable and lets Phase 3.5's CI pressure focus on the payload change. |
| 15 | `ChunkType` enum | Replace with typed ACP payloads | **A\*** | **Phase 3.5** | Follows #14. |

---

## 2. Fold-vs-split decision

**FOLD into Phase 3 (rows 1, 3, 4, 5, 6, 7, 10):** Six low-coupling cleanups consistent with the existing Phase 3 scope (rekeying + plane-separation preparation). Adds ~200 LOC net; Phase 3 stays single-revertable.

**SPLIT into Phase 3.5 (rows 14, 15):** Chunk payload redesign is a distinct concern. Rationale:
- Row rekeying (Phase 3) is *field-level* — same information, different key.
- Payload redesign (Phase 3.5) is *semantic* — different information shape. Consumers go from `chunk.type === 'tool_call'` + `JSON.parse(chunk.content)` to pattern-matching `SessionUpdate` variants.
- Independent revert value: if Flamecast or example consumers break on the payload change, Phase 3.5 reverts without unwinding the rekeying work.
- CI isolation: Phase 3.5's failure mode is different (consumer semantics) from Phase 3's (stream rekeying correctness).

**REJECT (rows 11, 12):** `StateHeaders` / `StateEnvelope` stay as private module helpers. Not consumer-visible.

---

## 3. Phase 3.5 — new section for execution plan

**A. Files touched**
- `crates/fireline-harness/src/state_projector.rs`
- `crates/fireline-tools/` (if any MCP chunk emission still uses the old shape)
- `packages/state/src/schema.ts`
- `packages/state/src/collections/turn-chunks.ts`
- `examples/flamecast-client/server.ts`
- `examples/multi-agent-team/`, `examples/live-monitoring/`
- `tests/state_fixture_snapshot.rs`

**B. Exact change summary**
- Replace `ChunkRow.chunk_type: ChunkType` + `ChunkRow.content: String` with `ChunkRow.update: sacp::schema::SessionUpdate` (or a thin typed wrapper if serde shape requires it).
- Delete `ChunkType` enum.
- Update `packages/state/src/schema.ts` chunk schema to the typed-update shape with ACP SDK SessionUpdate types.
- Update Flamecast and example consumers to pattern-match on `SessionUpdate` variants instead of `type`/`content` pairs.
- Provide a one-phase TS migration helper (e.g., `extractChunkTextPreview(update)`) so example code doesn't regress on the "show a text string" use case.

**C. Type-level change summary**
- Rust: `ChunkRow.{chunk_type, content} -> ChunkRow.update: SessionUpdate`
- TS: `ChunkRow['type' | 'content']` removed; `ChunkRow['update']` typed from `@agentclientprotocol/sdk`

**D. Tests**
- Update `tests/state_fixture_snapshot.rs` to assert new chunk shape.
- Add a regression test in Flamecast-style consumer code that a typed `SessionUpdate.ToolCall` variant renders the same logical transcript as before.

**E. Verification gate**
```bash
cargo test --workspace
pnpm --filter @fireline/state test
pnpm --filter @fireline/client test
rg -n "ChunkType|chunk\.type|chunk\.content" crates packages tests examples
```

The `rg` command should return only canonical-typed uses — no string-typed `chunk.type === '...'` comparisons.

**F. LOC estimate:** 250-400

**G. Dependencies:** Phase 3.

**H. Can be dispatched independently?** After Phase 3 lands.

**Rollback:** revert Phase 3.5 only; Phase 3's rekeyed rows still work with the old string-typed payload reader path because Phase 3 keeps the dual-write `chunk_v2` under the old shape until Phase 3.5 lands. (Caveat: if Phase 3's dual-write uses the new shape, Phase 3.5 is not independently revertable — flag this to Opus 1 when dispatching. Simplest resolution: Phase 3's dual-write writes the OLD string-typed shape under `chunk_v2` key; Phase 3.5 rewrites `chunk_v2` shape to typed SessionUpdate.)

---

## 4. Invariant conflict check

Cross-checked against `verification/spec/managed_agents.tla` canonical-ids invariants and `durable-subscriber-verification.md` DSV-* register:

| Invariant | Phase 3 impact | Phase 3.5 impact | Status |
|---|---|---|---|
| `AgentLayerIdentifiersAreCanonical` | Satisfied — all deletions remove SyntheticIdFields | Satisfied | GREEN |
| `InfrastructureAndAgentPlanesDisjoint` | Strengthened — HostInstanceRow moves off agent plane | Unchanged | GREEN |
| `CrossSessionLineageIsOutOfBand` | Satisfied — trace_id/parent_prompt_turn_id deleted | Unchanged | GREEN |
| `ChunkOrderingFromStreamOffset` | Satisfied — chunk_id/chunk_seq/seq deleted, ordering from offset | Unchanged | GREEN |
| `ApprovalKeyedByCanonicalRequestId` | Unaffected (Phase 2 scope) | Unchanged | GREEN |
| `SessionDurableAcrossRuntimeDeath` (wake) | Unaffected | Unchanged | GREEN |
| `DSV-01..05` / `DSV-10..13` (subscriber) | DurableSubscriber implementation is post-Phase-5; not blocking | Unchanged | N/A |

**No invariant conflicts.** Phase 3 and Phase 3.5 are both invariant-strengthening or invariant-neutral.

Flag: TLC should be run against `ManagedAgentsCanonicalIds.cfg` before Phase 3 dispatches (already my standing followup with w17 from the earlier TLA review). If TLC surfaces vacuous invariants, update the spec first, then dispatch Phase 3.

---

## 5. Phase 6 TypeScript cascade

Phase 3 row changes that mirror into `@fireline/state` Phase 6 migration:

| Phase 3 change | Phase 6 TS work | Coordination |
|---|---|---|
| `PromptTurnRow → PromptRequestRow` | `promptTurns → promptRequests` collection rename; shape migrates to `(sessionId, requestId)` key | Already in §Phase 6.B |
| `ChunkRow` rekey | `chunks` or `turnChunks` collection rekey by `(sessionId, requestId, toolCallId?)` | Already in §Phase 6.B; flag that Phase 3 dual-writes under `chunk_v2` entity name |
| `ConnectionRow` delete | `createConnectionTurnsCollection` delete; `connections` collection drop | Already in §Phase 6.B |
| `HostInstanceRow` move | `hosts` / `runtimeInstances` move to admin-only API | Aligns with §Phase 7 (Plane Separation) |
| `PendingRequestRow` delete | `pendingRequests` collection drop | NEW — flag to Phase 6 codex: add this deletion to the Phase 6 scope |
| `stop_reason: StopReason` retype (#3) | `stopReason` schema tightens from `z.string()` to the SDK's `StopReason` type | NEW — flag to Phase 6: update `promptRequestSchema.stopReason` to the typed variant |

Phase 3.5 cascade:

| Phase 3.5 change | Phase 6 TS work | Coordination |
|---|---|---|
| `ChunkRow` payload → typed `SessionUpdate` | `chunkSchema` rewrite to import `SessionUpdate` from `@agentclientprotocol/sdk`; consumers updated; example code updated | Requires Phase 6 to land AFTER Phase 3.5, OR Phase 3.5 coordinates the TS rewrite in the same PR |

**Recommendation:** Phase 3.5 coordinates the TS + Flamecast + examples rewrite in the same commit (it's already multi-package per §3.A above). Phase 6 absorbs the other residual changes.

---

## 6. Execution plan update

Minimal patch to `docs/proposals/acp-canonical-identifiers-execution.md`:

1. **§Phase 3.B** — append sub-bullets for audit adoptions:
   - Delete `ConnectionRow`, `ConnectionState`.
   - Move `HostInstanceRow` + `HostInstanceState` to the infra plane (`hosts:tenant-{id}` stream).
   - Delete `PendingRequestRow`, `PendingRequestDirection`, `PendingRequestState` from the agent-plane projection.
   - Delete `TraceEndpoint`.
   - Retype `stop_reason: Option<String>` → `Option<sacp::schema::StopReason>` on the renamed `PromptRequestRow`.
   - Trim `PromptTurnState` to only the variants the projector actually emits (Active, Completed, Broken unless TLA+ requires others).
   - Mark `StateHeaders` / `StateEnvelope` as out of scope for Phase 3 (keep as private module helpers).

2. **New §Phase 3.5 — Chunk Payload Redesign** — insert between §Phase 3 and §Phase 4. Use the contents of §3 above verbatim.

3. **§Phase 4 dependency update** — depends on Phase 3.5 instead of Phase 3.

PM: once this review lands, please patch the execution plan per the above and redispatch the Phase 3 codex with the expanded scope + invariant-conflict-clean gate.

---

## 7. Pre-dispatch checklist for Phase 3

- [ ] TLC manual run against `ManagedAgentsCanonicalIds.cfg` green (followup with w17 outstanding).
- [ ] Execution plan §Phase 3.B patched per §6 above.
- [ ] Phase 3.5 section appended per §3.
- [ ] Phase 6 scope updated with `PendingRequestRow` deletion and `stopReason` typed variant.
- [ ] Dispatch prompt explicitly instructs the codex to preserve `StateHeaders` / `StateEnvelope` as private.
- [ ] Dispatch prompt explicitly states the `text: Option<String>` preview field stays and is annotated as derived.
- [ ] Dispatch prompt confirms Phase 3 dual-writes under `prompt_request` / `chunk_v2` / `session_v2` entity names with the OLD payload shape for chunks, leaving Phase 3.5 to rewrite `chunk_v2` payload independently.

---

## 8. `child_session_edge.rs` — pull deletion up from Phase 5 into Phase 3

Opus 1 surfaced `crates/fireline-orchestration/src/child_session_edge.rs` as pre-canonical tech debt during this synthesis. Consumer grep confirms the debt:

**Writers + wiring (all in Rust):**
- `crates/fireline-orchestration/src/child_session_edge.rs` — the SHA256-minting writer + `ChildSessionEdgeRow` serialization
- `crates/fireline-tools/src/peer/mcp_server.rs:179` — calls `emit_child_session_edge(...)` on peer prompt calls
- `crates/fireline-tools/src/peer/lookup.rs` — defines `ChildSessionEdgeSink` trait + `ChildSessionEdgeInput` struct
- `crates/fireline-tools/src/peer/mod.rs`, `crates/fireline-host/src/bootstrap.rs`, `crates/fireline-harness/src/host_topology.rs` — thread the sink

**Agent-plane readers: NONE.**
- No Rust code reads `child_session_edge` rows off the state stream.
- No example reads `childSessionEdges` collection.
- No Flamecast consumer reads edge rows.
- Only surface usage is `packages/state/src/schema.ts` declaring the type + two tests (`packages/state/test/schema.test.ts:52`, `packages/state/test/rust-fixture.test.ts:33`) asserting the schema round-trips. The schema exists, the data is written, but nothing consumes it.

**Architectural problems:**
1. **Pre-canonical synthetic identity.** `edge_id = SHA256(parent_host_id + parent_session_id + parent_prompt_turn_id + child_host_id + child_session_id)` mints a Fireline identifier from a mix of infra-plane and agent-plane values. Violates canonical-ids acceptance criterion ("no bespoke lineage or edge tables independent of ACP identifiers and `_meta`").
2. **Plane-muddying.** A single row holds infra-plane (`parent_host_id`, `child_host_id`) and agent-plane (`parent_session_id`, `child_session_id`) identifiers together. Violates `InfrastructureAndAgentPlanesDisjoint`.
3. **Write-only emission.** No consumer — the stream is paying write cost for no read benefit.

**Why this was scheduled for Phase 5 originally:** the execution plan §Adjusted Phase Order said "Deleting the synthetic lineage structures first would break peer calls before a canonical replacement exists." That rationale was about `ActiveTurnIndex` (peer lookup genuinely depends on it); it does NOT apply to `child_session_edge` because nothing reads the edge rows. Deleting the writer is a pure deletion with no replacement needed.

**Decision: FOLD edge-writer deletion into Phase 3.** Pull it up from Phase 5.

**Additional Phase 3 scope:**
- Delete `crates/fireline-orchestration/src/child_session_edge.rs` (entire module).
- Delete `ChildSessionEdgeSink` trait + `ChildSessionEdgeInput` struct from `crates/fireline-tools/src/peer/lookup.rs`.
- Delete the `emit_child_session_edge(...)` call in `crates/fireline-tools/src/peer/mcp_server.rs`. Peer calls still function — they just stop emitting observational-only lineage rows.
- Remove `child_session_edge_sink` field + wiring from `PeerComponent` (`crates/fireline-tools/src/peer/mod.rs`), `HostTopologyContext` (`crates/fireline-harness/src/host_topology.rs`), and `crates/fireline-host/src/bootstrap.rs`.
- Delete the `ChildSessionEdgeWriter` module-level test in `child_session_edge.rs`.

**Phase 5 scope reduction:** §Phase 5 now deletes ONLY `ActiveTurnIndex` + residual peer-lineage refactor (which is still Phase-5-correct because peer lookup genuinely depends on canonical replacements that Phase 4 introduces). Remove `child_session_edge` from Phase 5's file list.

**Phase 6 TS cascade (unchanged):** `packages/state/src/schema.ts` `childSessionEdgeSchema` + `childSessionEdges` collection deletion stays in Phase 6. Between Phase 3 (Rust writer deleted) and Phase 6 (TS schema deleted), `childSessionEdges` collection exists but is silently empty. That's acceptable transitional state; update `packages/state/test/rust-fixture.test.ts:33` to stop expecting `child_session_edge` entities in the Phase 3 dispatch as a small coordinated change.

**Invariant impact:** strengthens `InfrastructureAndAgentPlanesDisjoint` (removes the worst offender) and reduces `SyntheticIdFields` surface. No conflicts. TLC should be re-run after this change to confirm still-green.

**Updated Phase 3 LOC estimate:** ~700 → ~850. Still within a single revertable phase.

---

## References

- [state-projector-surface-audit.md](../proposals/state-projector-surface-audit.md)
- [acp-canonical-identifiers.md](../proposals/acp-canonical-identifiers.md)
- [acp-canonical-identifiers-execution.md](../proposals/acp-canonical-identifiers-execution.md)
- [acp-canonical-identifiers-verification.md](../proposals/acp-canonical-identifiers-verification.md)
- [durable-subscriber-verification.md](../proposals/durable-subscriber-verification.md)
- [verification/spec/managed_agents.tla](../../verification/spec/managed_agents.tla)
