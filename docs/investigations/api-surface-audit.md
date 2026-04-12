# API Surface Audit — Wire Format Consistency

> **Scope:** HTTP API, ACP wire format, durable-stream event schemas, TS↔Rust type alignment, doc-comment freshness.
> **Anchor:** `origin/main` at `8ea5172` on 2026-04-12.

---

## Category A — HTTP API inconsistencies

### A1. `ProvisionRequest` vs legacy `CreateRuntimeSpec` field-set drift

**File:** `crates/fireline-host/src/router.rs` — `ProvisionRequest` struct
**File:** `packages/client/src/host.ts` (legacy backward-compat `createHostClient`)
**Severity: P1**

The legacy `host.ts` backward-compat shim at `packages/client/src/host.ts:308-318` sends `provider`, `host`, `port`, and `peerDirectoryPath` in the `POST /v1/runtimes` body. The Rust router's `ProvisionRequest` struct (which the handler deserializes into) does NOT declare those fields — serde silently ignores them. The actual provisioning uses the control-plane's server-side defaults for provider/host/port.

The Tier 5 client at `packages/client/src/host-fireline/client.ts` sends only the fields `ProvisionRequest` accepts (`name`, `agentCommand`, `resources`, `stateStream`, `topology`), so the **live demo path is correct**. The legacy shim is the only caller with the mismatch.

**Fix:** Either extend `ProvisionRequest` to accept `provider`/`host`/`port`/`peerDirectoryPath` (and pass them through to `ProvisionSpec` in the handler), or update the legacy shim in `host.ts` to stop sending them and add a comment documenting the narrowing.

### A2. `FirelineRuntimeDescriptor` drops 8 response fields

**File:** `packages/client/src/host-fireline/client.ts:37-42`
**Severity: P2**

The TS `FirelineRuntimeDescriptor` type declares only `runtimeKey`, `status`, `acp?`, `state?`. The Rust `HostDescriptor` response always includes 10 additional fields (`runtimeId`, `nodeId`, `provider`, `providerInstanceId`, `helperApiBaseUrl`, `createdAtMs`, `updatedAtMs`). These are silently dropped by JSON parsing (extra fields are ignored).

Functionally harmless — the client only needs `runtimeKey` and `status` for its logic — but the `acp` and `state` fields are marked optional (`?`) in TS while required in Rust. The post-`37db346` `HostHandle` type carries `acp` + `state`, so downstream callers rely on them being present.

**Fix:** Expand `FirelineRuntimeDescriptor` to include all `HostDescriptor` fields, or at minimum mark `acp` and `state` as required (drop the `?`).

### A3. Error response shape is consistent

All routes in `router.rs` return errors as `(StatusCode, Json<ErrorResponse>)` where `ErrorResponse = { error: String }`. No mixed error shapes found. **No issue.**

### A4. Serde rename_all consistency

All request/response structs use `#[serde(rename_all = "camelCase")]`. Enums (`HostStatus`, `SandboxProviderKind`) use `#[serde(rename_all = "snake_case")]` for variant tags. Two fields use explicit `#[serde(rename = "runtimeKey")]` / `#[serde(rename = "runtimeId")]` to preserve backward-compatible wire names while using `host_key` / `host_id` in the Rust struct. **No inconsistency** — all casing is intentional.

---

## Category B — Durable stream event schema drift

### B1. `child_session_edge` field names: Rust writes `parentHostId`, TS expects `parentRuntimeId`

**File (writer):** `crates/fireline-orchestration/src/child_session_edge.rs:20-32`
**File (reader):** `packages/state/src/schema.ts:106-115`
**Severity: P0**

The Rust struct `ChildSessionEdgeRow` was renamed during the Host vocabulary alignment:
```rust
#[serde(rename_all = "camelCase")]
struct ChildSessionEdgeRow {
    parent_host_id: String,    // wire: parentHostId
    child_host_id: String,     // wire: childHostId
    // ...
}
```

The TypeScript Zod schema still expects the pre-rename field names:
```typescript
parentRuntimeId: z.string(),   // expects: parentRuntimeId
childRuntimeId: z.string(),    // expects: childRuntimeId
```

**Impact:** Any TS consumer reading `child_session_edge` envelopes from the durable stream via `@fireline/state` collections will get `undefined` for both fields. The `useLiveQuery(childSessionEdges(...))` call in the browser harness's State Explorer "edges" tab will show empty parent/child columns.

**Fix:** Either (a) add `#[serde(rename = "parentRuntimeId")]` / `#[serde(rename = "childRuntimeId")]` to the Rust struct fields to preserve the old wire names (preferred — avoids breaking existing streams), or (b) update the TS Zod schema to match the new Rust field names (`parentHostId`, `childHostId`). Option (a) is safer because it maintains wire backward-compat with existing stream data.

### B2. All other event entity types match

Comprehensive cross-reference of every entity type written and read:

| Entity type | Writer | Reader | Wire field match | Status |
|---|---|---|---|---|
| `session` | `fireline-harness/state_projector.rs` | `fireline-session/session_index.rs` + `packages/state/schema.ts` | ✓ | OK |
| `prompt_turn` | `fireline-harness/state_projector.rs` | `fireline-session/active_turn_index.rs` + `packages/state/schema.ts` | ✓ | OK |
| `connection` | `fireline-harness/state_projector.rs` | `packages/state/schema.ts` (no Rust reader) | ✓ | OK (TS-only consumer) |
| `chunk` | `fireline-harness/state_projector.rs` | `packages/state/schema.ts` (no Rust reader) | ✓ | OK |
| `pending_request` | `fireline-harness/state_projector.rs` | `packages/state/schema.ts` (no Rust reader) | ✓ | OK |
| `runtime_instance` | `fireline-harness/state_projector.rs` | `fireline-session/host_index.rs` + `packages/state/schema.ts` | ✓ | OK |
| `runtime_spec` | `fireline-sandbox/stream_trace.rs` | `fireline-session/host_index.rs` + `session_index.rs` | ✓ (opaque value) | OK |
| `runtime_endpoints` | `fireline-sandbox/stream_trace.rs` | `fireline-session/host_index.rs` | ✓ (opaque `HostDescriptor`) | OK |
| `child_session_edge` | `fireline-orchestration/child_session_edge.rs` | `packages/state/schema.ts` | **✗ MISMATCH** | **P0 — see B1** |
| `permission` | `fireline-harness/approval.rs` | Custom inline polling (approval.rs) + `packages/state/schema.ts` | ✓ | OK (custom reader, not `StateProjection`) |
| `tool_descriptor` | `fireline-tools/tools.rs` | Not materialized (one-shot emit) | ✓ | OK |
| `fs_op` | `fireline-resources/fs_backend.rs` | Inline reader in same file | ✓ | OK |
| `runtime_stream_file` | `fireline-resources/fs_backend.rs` | Inline reader in same file | ✓ | OK |
| `resource_published` / `updated` / `unpublished` | `fireline-resources/publisher.rs` | Resource discovery index | ✓ (ResourceEvent envelope, not state-protocol) | OK |

### B3. Vocabulary rename did NOT break wire event-type strings

All wire event-type strings (`"session"`, `"runtime_spec"`, `"runtime_instance"`, `"runtime_endpoints"`, `"prompt_turn"`, etc.) are unchanged. The vocabulary rename (`RuntimeHost → HostDescriptor`, `create → provision`, etc.) was a Rust struct-level rename that preserved the serde wire names via `#[serde(rename = "runtimeKey")]`-style attributes. **No existing streams are broken by the rename.**

### B4. `runtime_instance` reader field-name mapping is correct but confusing

**File:** `crates/fireline-session/src/host_index.rs:71-80`

The reader struct `HostInstanceRecord` has a field `host_name: String` with `#[serde(rename = "runtimeName")]`. The writer in `fireline-harness/state_projector.rs` writes `runtimeName` via `#[serde(rename_all = "camelCase")]`. This works correctly on the wire, but the Rust field name (`host_name`) and the wire name (`runtimeName`) don't match semantically, which is confusing for maintenance.

**Severity: P2.** **Fix:** Either rename the Rust field to `runtime_name` (matching the wire) or document the intentional divergence.

---

## Category C — TS ↔ Rust type mismatches

### C1. Sandbox types: ToolCall / ToolCallResult shapes differ

**File (Rust):** `crates/fireline-sandbox/src/primitive.rs`
**File (TS):** `packages/client/src/sandbox/index.ts`
**Severity: P1 (aspirational — no live wire path today)**

| Type | Rust field | TS field | Wire match |
|---|---|---|---|
| `ToolCall.name` | `name` | `tool_name` | ✗ |
| `ToolCall.input` | `input` | `arguments` | ✗ |
| `ToolCall` (TS only) | — | `call_id?` | TS-only |
| `ToolCallResult` (Rust) | `{ output, exit_status? }` | — | Flat struct |
| `ToolResult` (TS) | — | `{ kind: 'ok'\|'error', value\|message }` | Discriminated union |

These are incompatible wire shapes. **However, no TS→Rust Sandbox wire path exists today.** The `MicrosandboxSandbox` impl is Rust-only; the TS `Sandbox` interface exists as an aspirational public API for future TS-side sandbox satisfiers. If a TS satisfier is ever wired to a Rust sandbox over HTTP/RPC, this will break.

**Fix:** Align the TS `ToolCall` / `ToolResult` types with the Rust equivalents, or document the intentional divergence as "TS sandbox types describe the client-facing contract; Rust types describe the satisfier's internal shape."

### C2. `HostStatus` public API vs wire format — correctly bridged

**File (Rust):** `crates/fireline-session/src/host_identity.rs` — `HostStatus` enum with `#[serde(rename_all = "snake_case")]`
**File (TS public):** `packages/client/src/host/index.ts` — discriminated union `{ kind: 'created' | 'running' | 'idle' | ... }`
**File (TS client):** `packages/client/src/host-fireline/client.ts:114-130` — `mapRuntimeStatus()` function

The public API types and Rust wire types use different naming (`created` vs `starting`, `running` vs `ready`), but `mapRuntimeStatus()` explicitly bridges between them. This is working as designed.

**Severity: informational — NOT a bug.** The two layers are intentionally different (wire layer is Rust-native, public API is primitive-layer semantic). `mapRuntimeStatus()` is the adaptation point.

### C3. ResourceSourceRef variants — correctly aligned

**File (Rust):** `crates/fireline-resources/src/resource.rs`
**File (TS):** `packages/client/src/core/resource.ts`

Both define 9 identical variants: `localPath`, `s3`, `gcs`, `dockerVolume`, `durableStreamBlob`, `streamFs`, `ociImageLayer`, `gitRepo`, `httpUrl`. Inner field names match via `#[serde(rename_all = "camelCase")]` on the Rust side and camelCase literal names on the TS side.

The Rust `r#ref` raw-identifier field (in `GitRepo`) serializes as `"ref"` on the wire — matching the TS `ref` field. **No mismatch.**

### C4. ProvisionSpec — intentionally different layers

**File (Rust):** `crates/fireline-sandbox/src/lib.rs` — internal `ProvisionSpec` (the full Rust-side runtime provisioning spec with `host`, `port`, `provider`, `durable_streams_url`, etc.)
**File (TS public):** `packages/client/src/host/index.ts` — `ProvisionSpec` (the Host-primitive-layer payload with `topology?`, `resources?`, `agentCommand?`, `metadata?`)

These are intentionally different: the TS `ProvisionSpec` is what a caller passes to `host.provision()`, and the `createFirelineHost` satisfier translates it into the HTTP `ProvisionRequest` body that the Rust router accepts. The Rust internal `ProvisionSpec` includes infrastructure fields (`host`, `port`, `durable_streams_url`) that are server-side concerns, not client-facing.

**Severity: informational — NOT a bug.** The layering is correct; the satisfier layer does the mapping.

---

## Category D — Stale or misleading doc comments

### D1. `fireline-session/src/host_index.rs` — 5 references to dissolved `fireline-conductor`

**Lines:** 4, 9, 29, 56-57, 70
**Severity: P2**

Top-of-file `//!` comment and inline `///` comments reference paths like `crates/fireline-conductor/src/trace.rs:134` and `crates/fireline-conductor/src/state_projector.rs`. The `fireline-conductor` crate was dissolved during the restructure. These should reference `crates/fireline-harness/src/` (where the state projector and trace modules now live).

**Fix:** Rewrite the 5 path references to use `crates/fireline-harness/src/...`. The module-level description is otherwise accurate.

### D2. `fireline-harness/src/audit.rs:7,17` — references dissolved `fireline_conductor` and `fireline_components`

**Line 7:** `//! existing [fireline_conductor::trace::DurableStreamTracer]`
**Line 17:** Example code references `use fireline_components::audit::...`
**Severity: P2**

Both should be `crate::trace::DurableStreamTracer` and `use fireline_harness::audit::...` respectively.

### D3. `fireline-sandbox/src/satisfiers.rs:8`, `microsandbox.rs:10,34` — reference dissolved `fireline_conductor::primitives`

**Severity: P2**

Three `//!` comments reference `fireline_conductor::primitives::Host` and `fireline_conductor::runtime::ResourceMounter`. These are now in `fireline-session::host_identity` (or wherever the Host types landed post-restructure) and `fireline-resources`.

**Fix:** Replace module paths with generic architecture-level references or use the current crate paths.

### D4. `fireline-host/src/bootstrap.rs` — top-of-file doc comment is stale

**Severity: P2**

The audit at `8613304` already flagged this (audit §Q6 lines 143-162): the doc comment describes routes and embedded stream hosting that changed during the restructure. The file's actual behavior (what routes it merges, whether it owns embedded durable-streams) has drifted from the comment.

**Fix:** Rewrite the top-of-file `//!` block to describe what `bootstrap::start` actually does post-restructure.

### D5. `tests/managed_agent_orchestration.rs:62` — path reference to dissolved crate

**Severity: P2**

Test doc comment references `crates/fireline-conductor/src/runtime/mod.rs`. Should reference the current location in `fireline-host` or `fireline-sandbox`.

---

## Summary by severity

| Severity | Count | Category | Immediate action needed |
|---|---|---|---|
| **P0** | 1 | B1 — `child_session_edge` field names | **Yes** — TS consumers of the "edges" tab will see empty parent/child fields |
| **P1** | 2 | A1 — legacy client sends extra fields; C1 — Sandbox type shape divergence | Pre-demo for A1 (check if any caller depends on the legacy path); track for C1 |
| **P2** | 10+ | A2, B4, D1-D5 — dropped response fields, confusing field-name mapping, stale doc comments | Post-demo cleanup batch |

### P0 fix recommendation

**B1 is the only finding that can break a live consumer today.** The fix is a one-line change in Rust:

```rust
// crates/fireline-orchestration/src/child_session_edge.rs
#[serde(rename_all = "camelCase")]
struct ChildSessionEdgeRow {
    // ...
    #[serde(rename = "parentRuntimeId")]
    parent_host_id: String,
    // ...
    #[serde(rename = "childRuntimeId")]
    child_host_id: String,
    // ...
}
```

This preserves the wire format (`parentRuntimeId`, `childRuntimeId`) that the TS Zod schema and existing durable streams expect, while keeping the Rust field names at `parent_host_id` / `child_host_id` for internal code clarity. **Do not rename the wire fields to `parentHostId` — that would break existing stream data.**
