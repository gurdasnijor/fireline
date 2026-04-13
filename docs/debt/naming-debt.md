# Naming Debt

> Status: living catalog
> Started: 2026-04-12
> Scope: naming hygiene issues surfaced in reviews. Addressed AFTER canonical-ids Phase 8 closes and demo ships.

Naming debt is cheap to fix but compounds fast — the "fireline-host" / `run_host` collision cost ~10 min of confusion in a critical-review session. This doc catalogs the cases worth a rename pass. Cross-linked to beads where an epic exists.

## Rules for addressing

1. **Don't rename mid-refactor.** Wait for canonical-ids Phase 8 + DS Phase 2+ to clear.
2. **Batch per crate** — one PR per crate, not one PR per symbol. Reduces churn.
3. **Deprecation aliases allowed for public surface** (@fireline/client, @fireline/state). Internal Rust rename is hard break — land as one PR.
4. **Don't rename what's being deleted.** Canonical-ids transitional names (`PromptTurnRow`, `ChildSessionEdgeRow`, `ActiveTurnIndex`, etc.) are already scheduled for deletion — leave them.

---

## High-impact renames (post-Phase-8)

### N1. `fireline-host` crate conflates two planes

The crate contains both:
- `bootstrap.rs` — data-plane agent host boot (runs `/acp`, owns state stream, spawns agent subprocess)
- `control_plane.rs` — control-plane sandbox provisioner (runs `/v1/sandboxes`, no agent, no state stream)

These are different architectural roles with ~30 lines of shared HTTP plumbing. The crate name encourages the wrong mental model (they look like variants of one primitive).

**Fix:** split the crate.
- `fireline-agent-host` (data plane)
- `fireline-control-plane` (control plane)

Related: [hosting-primitives-critical-review-2026-04-12.md §R1'](../reviews/hosting-primitives-critical-review-2026-04-12.md).

### N2. `control_plane::run_host()` function name lies

The function lives in `control_plane.rs`, its file says "control plane," but the function says "host." It boots the control-plane sandbox-provisioner, not a host. Path `fireline_host::control_plane::run_host` contains two different meanings of "host" colliding.

**Fix:** rename to `control_plane::serve` (axum idiom) or `control_plane::run_control_plane`. If N1 lands first, becomes `fireline_control_plane::serve`.

**Priority:** HIGH — this is the specific naming collision that actively misleads.

### N3. `HostConfig` in control_plane.rs is for the control plane itself

```rust
// crates/fireline-host/src/control_plane.rs
pub struct HostConfig { ... }  // fields for the control-plane process
```

A reader sees `HostConfig` and expects "config for a host." But this is config for the provisioning service itself, and a separately-defined `BootstrapConfig` exists for the actual data-plane host.

**Fix:** `HostConfig` → `ControlPlaneConfig`. Lands with N1/N2.

### N4. `BootstrapConfig` is data-plane-specific

Nothing in the name signals "data-plane agent host." Generic word "bootstrap" misleads if/when a third boot path appears (Tier C subscriber host, etc.).

**Fix:** `BootstrapConfig` → `AgentHostConfig`. `BootstrapHandle` → `AgentHostHandle`. `bootstrap::start` → `agent_host::start`.

### N5. Host lifecycle event vocabulary overlap

Pre-existing smell documented in hosting review §S4. Two overlapping vocabularies:

- State stream: `host_spec_persisted`, `host_instance_started`, `host_instance_stopped`
- Deployment stream: `HostRegistered`, `HostProvisioned`, `HostHeartbeat`, `HostStopped`, `HostDeregistered`

`HostRegistered` + `HostProvisioned` are emitted back-to-back same timestamp. `HostStopped` + `HostDeregistered` + `host_instance_stopped` triple for one transition.

**Fix:** consolidate to three canonical verbs — `host.present`, `host.heartbeat`, `host.gone`. See hosting review §R4. Naming + vocabulary change together.

---

## Medium-impact renames

### N6. Two sources of "host" truth have the same name

- `fireline_session::HostIndex` — control-plane's provision-driven index
- Deployment-discovery stream `hosts:tenant-{id}` — data-plane self-registration

Both semantically mean "set of hosts" but populated from different sources. Neither name signals which view.

**Fix:** rename one to disambiguate. Options:
- `HostIndex` → `ProvisionedHostIndex` (tracks what control plane created)
- Deployment stream projection → `RegisteredHostIndex` (tracks what self-registered)

Or unify into one stream-backed read model (architectural change — out of naming-debt scope).

### N7. `advertised_state_stream_url` vs `state_stream_url` vs `FIRELINE_ADVERTISED_STATE_STREAM_URL`

Three variants of the same concept floating around:
- `host_identity.rs::PersistedHostSpec.advertised_state_stream_url`
- bootstrap.rs local `state_stream_url`
- Env var `FIRELINE_ADVERTISED_STATE_STREAM_URL`

Pick one: "advertised URL" (externally-reachable) vs "internal URL" (localhost). Consistent prefixing across Rust fields + env vars + CLI flags.

### N8. `SharedTerminal` name doesn't convey its role

It's the abstraction for one agent subprocess + its stdio pipes. "SharedTerminal" hints at terminal-sharing (multiple readers/writers?) but actually it's "one subprocess we proxy ACP to." Misleading.

**Fix:** `SharedTerminal` → `AgentSubprocess` or `AgentChannel`. Depends on what the upcoming "multiple agents per host" work (if any) wants.

### N9. `ComponentContext` is broader than "component"

Contains host identity, stream URLs, peer registry, producer handles, mounted resources. It's actually a "host runtime context" or "session environment." "ComponentContext" understates.

**Fix:** rename to `HostRuntimeContext` when it moves out of `fireline-harness` per hosting review §R2.

---

## Low-impact / cosmetic

### N10. `chrono_like_now_ms()` in bootstrap.rs

Literally says "chrono-like" (we chose not to use chrono). Just call it `now_ms()` or `unix_ms()`. One-line cleanup.

### N11. `@fireline/cli` package.json repository URL inheritance

Already fixed in PR #6 follow-up commit 244f7d6 — all package.jsons now point to `gurdasnijor/fireline`. Preserve this as a regression check: add CI assertion that `"repository"` in every package matches the canonical URL.

### N12. `base_components_factory` — factory for one element

Naming aside, this is YAGNI. Already flagged in hosting review §S9.

---

## Already-addressed (retroactive)

Keeping for audit trail:

- ✅ `approval_request_id` (SHA256 hash) → canonical ACP `RequestId` (Phase 2, commit `074b14e`)
- ✅ `_meta.fireline.traceId` / `parentPromptTurnId` → W3C `_meta.traceparent/tracestate/baggage` (Phase 4, commit `429475e`)
- ✅ `chunk_type + content:String` → typed `sacp::schema::SessionUpdate` (Phase 3.5, commit `714cc84`)
- ✅ `PromptTurnRow` → `PromptRequestRow` (Phase 3, commit `f9a4f74`)
- ✅ `fireline-semantics::ids` module → extracted to dedicated `fireline-acp-ids` crate
- ✅ `child_session_edge` rows → deleted (Phase 3, pulled up from Phase 5)
- ✅ `ChildSessionEdgeRow` TS schema → deleted (Phase 6)
- ✅ `spawn(bin, ...)` → `spawn(bin.path, ...)` in `fireline-agents.js` (PR #6 — silent bug)

---

## Dispatch guidance

**When canonical-ids Phase 8 closes AND demo ships:**

1. Create epic bead `mono-naming-debt` or similar. Link this doc.
2. Batch N1 + N2 + N3 + N4 into one "crate split + rename" PR. Mechanical; `pnpm --filter` + `cargo check` will flag all call sites.
3. N5 is a separate PR (event vocabulary consolidation) — requires coordinating with any deployment-stream consumers.
4. N6-N9 land opportunistically as touched-anyway changes.
5. N10-N12 are cosmetic; batch with unrelated low-risk cleanups.

No naming rename should block architectural work. Nothing here is a correctness issue.
