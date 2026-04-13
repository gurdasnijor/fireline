# Technical Debt

> Status: living catalog
> Started: 2026-04-12
> Scope: structural / architectural debt surfaced during the canonical-ids refactor + demo push. For addressing AFTER canonical-ids Phase 8 closes and demo ships.

Unlike naming debt, these items require design thinking and non-trivial implementation. Ranked by architectural cost.

## Rules for prioritizing

1. **HIGH** = architectural invariant weakened, or compound-cost (blocks/distorts future work). Address first.
2. **MEDIUM** = real smell, but not blocking. Schedule opportunistically when touched.
3. **LOW** = cosmetic debt. Defer unless easy-batched with other work.

Cross-referenced to existing beads where applicable.

---

## HIGH — architectural

### T1. Hosting-layer structural debt

Full inventory in [hosting-primitives-critical-review-2026-04-12.md](../reviews/hosting-primitives-critical-review-2026-04-12.md).

Headline items:
- **Layering inversion** (§S2): `fireline-host` depends on `fireline-harness` for host-orchestration helpers (`ComponentContext`, `AcpRouteState`, `emit_host_*`, `build_host_topology_registry`). Harness should be session-execution substrate only. Flip the direction.
- **Load-bearing event ordering** (§S3): `bootstrap.rs` line 226-230 hard-codes "emit stream events before materializer preloads" to work around `StateMaterializer` empty-stream exit. Fix the materializer; delete the ordering constraint.
- **Redundant event vocabulary** (§S4): 5 boot events + 3 shutdown events across 2 streams. Consolidate to 3 canonical verbs.
- **Unstructured shutdown** (§S5): any early failure skips remaining cleanup steps. Collect errors, always proceed.
- **Two sources of host truth** (§R7): control-plane `HostIndex` (provision-driven) vs data-plane discovery stream (self-registration). Document boundary or unify.

**Epic suggestion:** `mono-hosting-debt` after Phase 8.

### T2. `StateMaterializer` empty-stream exit

The materializer's preload exits the worker when it hits an empty stream, causing `"state materializer worker exited before preload completed"`. Every boot path works around it by pre-seeding events before calling preload.

**Fix options:**
- Tolerate empty stream; subscribe live and wait for first envelope.
- Preload blocks with a timeout for the first envelope.
- Expose `live_from_empty()` option.

Standalone patch, no dependencies on other tracks. Could ship earlier as a quick win.

### T3. Two views of host truth boundary unclear

Control-plane's `HostIndex` is populated by provision requests. Data-plane hosts self-register on the discovery stream. These are two different views of "hosts in the system" that never merge.

**Options:**
- Document the boundary: control-plane HostIndex is *provisioned-by-me*; discovery stream is *any-host-that-exists*.
- Unify: control plane subscribes to discovery stream and merges provision records with self-registrations.

Security trade-off: option 2 means untrusted self-registrations can appear in the control plane's view. Option 1 is safer but fragments observability.

### T4. `fireline.db()` and Phase 7 plane-separation completion

Phase 7 enforced plane separation in the Rust projector. The TS `@fireline/client::fireline.db()` surface should be re-verified end-to-end post-Phase-8: confirm zero infrastructure fields leak through the public read API. Bead: `mono-vkpp.9` (closed, but the TS assertion test should run in CI on every change to schema.ts).

Follow-up: the `db-plane-separation.test.ts` from Phase 7 should be a GATE, not a one-time check. Add to CI matrix.

---

## MEDIUM — structural

### T5. `ControlPlaneError` collapses anyhow → 500

`crates/fireline-host/src/router.rs::ControlPlaneError::From<anyhow::Error>` returns `StatusCode::INTERNAL_SERVER_ERROR` for any error. Providers (docker, local, anthropic) can signal quota/auth/conflict/not-found but all flatten to 500.

**Fix:** typed error taxonomy per provider category. Map to 409/404/403/502 where appropriate.

### T6. Demo code leakage into production paths

[`demo-code-leakage-audit-2026-04-12.md`](../reviews/demo-code-leakage-audit-2026-04-12.md) (commit `60d19ad`) flagged hacks bleeding from demo captures into production API+CLI. Needs a sweep.

### T7. `StateRight` coverage gaps

- `mono-vkpp.11` — QA-4 Stateright invariant regression (in progress).
- `mono-vkpp.12` — canonical-ids Stateright model.
- `mono-vkpp.13` — durable-subscriber Stateright model.

All three should land before DS active-profile phases (webhook / peer / timer) dispatch. Otherwise those phases lack mechanical verification of their DSV-03/04/05 obligations.

### T8. TS-Rust fixture tests carry transitional baggage

`packages/state/test/rust-fixture.test.ts` asserts specific entity shapes. Throughout the canonical-ids cascade, these assertions were updated piecewise. After Phase 8, do a clean audit: no transitional alias acceptances, fixtures match canonical shapes exactly.

### T9. `extractChunkTextPreview` TS helper is interim

Phase 3.5 shipped this helper to unblock example code during the chunk-payload migration. Intended to be dropped once all consumers pattern-match typed `SessionUpdate` directly.

**Action:** track as a deprecation in the @fireline/state package. Remove in the Phase 8 cleanup or a follow-up patch. Add a `@deprecated` JSDoc flag.

### T10. `@fireline/client` + `@fireline/state` ACP-id shims are byte-for-byte duplicates

Phase 1 landed identical `acp-ids.ts` in `@fireline/client` and `acp-types.ts` in `@fireline/state`. Slight duplication; interim. Either re-export one from the other, OR accept the duplication permanently if bundler tree-shaking concerns justify.

### T11. Deployment-stream consumer audit

The discovery stream (`hosts:tenant-{id}`) has multiple writers (data-plane hosts self-register) and readers (peer registry, future `DeploymentSpecSubscriber`, operator tooling). Who's consuming what? A consumer map would catch future schema changes before they break readers.

### T12. CLI `--repl` stub unimplemented

`packages/fireline/src/cli.ts` accepts `--repl` but only prints a message. Either implement an ACP REPL client (connect to running ACP endpoint, interactive prompt loop) or delete the flag. Currently misleads.

### T13. `fireline deploy --to <platform>` CLI verb deferred

Per `docs/proposals/fireline-cli-execution.md` reshape — `fireline deploy --to cloudflare|fly|k8s` was scoped as optional thin target-adapter wrapper. Not shipped. Decide: drop (users run `wrangler deploy` / `fly deploy` / `kubectl apply` directly) or implement.

### T14. `fireline push` (Tier C spec-stream publish) not implemented

Per `hosted-deploy-surface-decision.md §Tier C` — when multi-spec-per-host becomes a need, `fireline push <spec> --to <stream-url>` publishes to a durable-streams resource. DeploymentSpecSubscriber (Phase 6A) consumes. Not urgent; Tier C is deferred.

### T15. No Linux musl variant

PR #6 platform packages target `x86_64-unknown-linux-gnu` (glibc). Alpine (musl) installations of `@fireline/cli` would need a separate `@fireline/cli-linux-x64-musl` package. Add if/when demand surfaces.

---

## LOW — polish

### T16. `PersistedHostSpec` construction inline with hardcoded defaults

`bootstrap.rs:231+` has `provider: SandboxProviderRequest::Local` hardcoded, even though direct-host doesn't use the provider abstraction. `stream_storage: None`, `peer_directory_path: None` inline defaults. Extract to a builder or helper.

### T17. `connect_host()` IP rewrite wrong for containers

`bootstrap.rs` maps `0.0.0.0` → `127.0.0.1` in the advertised ACP URL. Unreachable from outside the container. Needs optional `FIRELINE_ADVERTISED_ACP_URL` env override in direct-host mode (already exists in control-plane mode).

### T18. `base_components_factory` YAGNI

`AcpRouteState.base_components_factory: Arc<dyn Fn() -> Vec<...>>` produces exactly one thing: `LoadCoordinatorComponent`. Replace with direct `Vec<DynConnectTo>` until a second consumer appears.

### T19. `agent_command_for_spec` clone workaround

`SharedTerminal::spawn` takes `Vec<String>` by value, forcing an upfront clone in bootstrap.rs. Change signature to `&[String]` or `Arc<[String]>`. Cosmetic.

### T20. `BootstrapHandle` public identity field duplication

`host_id`, `host_key`, `host_created_at` are public on the handle AND stored internally for shutdown event emission. Risk of divergence. Either `Arc<String>` shared, or make them accessor methods.

### T21. `BootstrapConfig.control_plane_url` dead field

Never read. Delete.

### T22. Per-target deployment validation checklist

Bead: `mono-8c5`. Post-demo residual. Each supported target (Fly, Railway, CF Containers, Docker Compose, K8s) needs a smoke-test validation against the three-bar requirement (long-running containers + persistent storage + HTTP/SSE).

### T23. DeploymentSpecSubscriber TLA gap

Bead: `mono-445` (R4). Before Phase 6A (Tier C) dispatches, extend DurableSubscriber TLA model to include `deployment_spec_published` / `spec_loaded` actions. Non-urgent; Tier C is deferred.

### T24. Observability post-demo phases

Per `docs/proposals/observability-integration.md`, Phases 3/4 extend span coverage beyond the demo-minimum 5 spans. Not beaded yet. Create a follow-up epic after demo.

### T25. ACP Registry Phase 3+ follow-ons

`agent_catalog` Phase 1-3 landed. Future work (caching, auth, fallback semantics beyond what's in `crates/fireline-tools/src/agent_catalog.rs`) not beaded. Define next scope when demand appears.

---

## Already-addressed (audit trail)

- ✅ Phase 7 `HostIndex` stale-Ready descriptor regression — fixed `bb8cd9d`
- ✅ Phase 3 `session/load` restart fix for embedded-spec Docker — fixed PR #4 (`ca79eab`)
- ✅ PR #6 repository URL on all platform packages — fixed `244f7d6`
- ✅ `resolve-binary` `spawn(bin, ...)` silent bug in `fireline-agents.js` — fixed in PR #6
- ✅ Phase 1 `fireline-semantics::ids` crate-location drift — extracted to `fireline-acp-ids`
- ✅ Phase 1 `@fireline/state` ACP-id shim gap — landed
- ✅ `child_session_edge` write-only tech debt — deleted in Phase 3
- ✅ Approval SHA256 `request_id` → canonical `RequestId` — Phase 2
- ✅ `InheritedLineage` / `_meta.fireline.*` → W3C trace context — Phase 4
- ✅ `ActiveTurnIndex` synthetic lineage bridge — deleted Phase 5
- ✅ TS schema `.passthrough()` during migration window → `.strict()` Phase 8

---

## Dispatch guidance

**After demo + Phase 8 close:**

1. Epic beads:
   - `mono-hosting-debt` — T1-T4 (or split T1 into sub-beads)
   - `mono-naming-debt` — parallel, low-risk, can start earlier
   - `mono-verification-gaps` — T7, T11
2. Per-bead HIGH items get dispatched in order of blast-radius. T2 (materializer) has the highest leverage per LOC changed.
3. LOW items stay in backlog; land opportunistically when you're touching the surrounding code anyway. Don't dispatch dedicated PRs for single-field cleanups.

**Cadence guidance:** treat this doc + `naming-debt.md` as a living catalog. When a new smell surfaces in a review, append rather than create a new doc. Delete entries as they close. Revisit quarterly or at each major phase boundary.
