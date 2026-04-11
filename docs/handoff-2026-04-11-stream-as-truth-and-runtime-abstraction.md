# Session Handoff — Stream-As-Truth Phase 1 & Runtime Abstraction Push

> **Created:** 2026-04-11 (late morning, immediately after the demo-day debt-paydown session)
> **Author:** Claude Opus 4.6 session (workspace:4, lead coordinator)
> **For:** the next session continuing the runtime-abstraction push and driving toward a full-featured demo
> **Companion handoffs:** `docs/handoff-2026-04-11-managed-agent-suite.md` (original demo-day starting point) and `docs/handoff-2026-04-11-post-debt-paydown.md` (post-arch-debt-paydown state)

This is the third handoff in the day's sequence. Read those two first for the full arc; this one focuses on what landed *after* the post-debt-paydown point and on the two directions gnijor named for the next phase: **pushing on the runtime abstraction** and **getting closer to a full-featured demo** — with **`superradcompany/microsandbox`** currently under evaluation as a candidate provider.

## TL;DR — where you're picking up

- **CI is green on `0af8612`.** 32 managed-agent-suite tests pass on CI (30 primary + 2 `runtime_index_agreement`). 3 tests remain intentionally ignored as Docker-scoped cross-reference markers.
- **Stream-as-truth refactor is mid-sequence.** Steps 0, 0.5, and 1 are landed. Steps 2 (flip the read path in the production control plane) and 3 (delete `RuntimeRegistry` + `HeartbeatTracker` + stale scanner) are explicitly deferred — they require adding shared-state-stream subscription to the production control plane, which is non-trivial infra.
- **One real production bug remains from Step 1.** A parallel agent's diagnosis nailed the durable-streams producer-id dedup trap; I fixed `runtime_endpoints_persisted` but `runtime_spec_persisted` is still using a reused producer id. Currently safe in practice only because every spec call writes the same body. See §"Known production bugs" below.
- **`RuntimeHost` split proposal is written and ready.** `docs/proposals/runtime-host-split.md` plus its stream-as-truth interaction appendix (commit `3244eb7`). It has a 5-commit transition plan. Stream-as-truth Steps 2/3 will *subsume* parts of it — do them first.
- **`packages/client` gained `resume(sessionId)`.** workspace:10 shipped `36096b7` — 214 lines of TS + 195 lines of vitest coverage. Orthogonal to the Rust churn. Good reference for the TS surface area.
- **Runtime abstraction direction is named.** gnijor is evaluating `superradcompany/microsandbox` as a candidate runtime provider. See §"Runtime-abstraction push" below for where that slots in.

## Commits this session (newest first)

```
0af8612 Defer Docker CI step pending image build caching
5a07990 Use unique producer id per runtime_endpoints emit
36096b7 Add resume(sessionId) helper to @fireline/client HostClient   (workspace:10)
3455f3c Run control_plane_docker and runtime_index_agreement in CI     (superseded by 0af8612 for Docker)
93adcf4 Add runtime_endpoints envelope + projection (stream-as-truth step 1)
bf67bb7 Emit runtime_spec_persisted from direct-host bootstrap
654ffaf Flush runtime_instance_stopped on shutdown + add agreement test
7c3b81f Add RuntimeIndex projection for stream-derived runtime view
8cc07ed Formalize stream-as-truth runtime index invariant               (workspace:6 - TLA/Stateright)
3244eb7 Update runtime-host-split proposal with stream-as-truth interaction (workspace:10)
4cc6c75 Fix transient 409 flake on orchestration live-resume             (workspace:9 — SharedTerminal grace-period retry)
91bbb0b Formalize unified liveness invariant in semantic kernel          (workspace:6)
053209b Add session handoff doc for post-debt-paydown state              (workspace:8)
54f4364 Propose RuntimeHost split (debt report item 2)                   (workspace:10)
6045c4a Unify control-plane liveness ownership                           (workspace:9 — will be DELETED by stream-as-truth step 3)
dcf93bf Fix review findings from 8301c61 and 0a30269
b82f642 Add fireline-semantics crate and verification spec with Stateright checks (workspace:6)
d3d7e7f Replace architecture doc with comprehensive reference from arch2 synthesis (workspace:10)
0a30269 Scope down AppState and tighten the last three lib.rs modules
4eaf94a Add resources field and ResourceRef type to @fireline/client host API
```

Every commit carries a `Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>` trailer. Preserve the convention.

## Stream-as-truth — where we are in the sequence

The thesis: **delete `RuntimeRegistry` entirely and derive runtime existence from durable state stream envelopes.** Heartbeats become optional liveness hints, not source of truth. Control plane is a stateless reader that materializes a `RuntimeIndex` projection.

### ✅ Landed (Steps 0, 0.5, 1)

**`src/runtime_index.rs`** — new projection alongside `SessionIndex`. Materializes three independent maps from the shared state stream:

- `runtime_specs` (keyed by `runtime_key`, from `runtime_spec` envelopes)
- `runtime_instances` (keyed by `runtime_id`, from `runtime_instance` envelopes the child process emits at start/stop)
- `runtime_endpoints` (keyed by `runtime_key`, from `runtime_endpoints` envelopes the control plane emits at every mutation — the Step 1 addition)

The full `RuntimeDescriptor` surface (runtime_id, node_id, acp.url, state.url, helper_api_base_url, provider_instance_id, status, timestamps) is now mirrored to the stream on every `create`/`register`/`stop` call in `crates/fireline-conductor/src/runtime/mod.rs`. The direct-host path in `src/bootstrap.rs` emits `runtime_spec` too (commit `bf67bb7`) but does **not** emit `runtime_endpoints` — direct-host has no external registrar to advertise to.

**`tests/runtime_index_agreement.rs`** — two integration tests that assert the stream-derived projection observes the same runtime lifecycle that `RuntimeRegistry` observes. These are the empirical proof that the thesis holds for the control-plane-managed path. They run in CI.

**Formal counterparts** — `crates/fireline-semantics/src/stream_truth.rs` + `verification/stateright/src/lib.rs` from workspace:6's commit `8cc07ed` prove the projection invariant on the TLA/Stateright side. Formal + empirical agreement was a nice signal.

### ⏸️ Deferred (Steps 2, 3, 4)

**Step 2 — Flip the read path in the production control plane.** Blocker: the production control plane (`crates/fireline-control-plane/src/main.rs`) has **no shared-state-stream subscription** today. Tests use `ControlPlaneHarness` which has one, but prod doesn't. Adding it requires:

1. A new `--shared-state-stream-url` CLI flag (or equivalent config) on the control plane binary
2. Spawning a `RuntimeMaterializer` with a `RuntimeIndex` subscribed to that URL at startup
3. Passing the same URL to every runtime it spawns as `FIRELINE_EXTERNAL_STATE_STREAM_URL` (already passed when `--shared-stream-base-url` is set via `local_provider.rs:146–151`)
4. Flipping `router::list_runtimes` and `router::get_runtime` to read from the index
5. Keeping writes going to `RuntimeRegistry` until Step 3, as rollback safety

The `RuntimeIndex` API is already in place (`endpoints_for`, `list_endpoints`) — those are explicitly the replacements for `RuntimeRegistry::get/list`.

**Step 3 — Delete `RuntimeRegistry` and friends.** With reads flipped and the agreement test green, delete:

- `RuntimeRegistry` (the write path becomes "emit to stream")
- `HeartbeatTracker` — workspace:9's liveness unification in `6045c4a` becomes **unnecessary and gets deleted** as part of this cut. That's a real cost-savings signal, not just cleanup.
- The stale scanner in `crates/fireline-control-plane/src/main.rs`
- `RuntimeHost::register` / `heartbeat` endpoint handlers collapse to "emit endpoints envelope" (or go away entirely if we accept that children don't need to POST back)

**Step 4 (optional cleanup)** — tighten the resulting surface; most of the `runtime-host-split` proposal's `RuntimeSpecJournal` and `RuntimeLifecycle` become redundant after Step 3, which is why the split proposal was explicitly updated (in `3244eb7`) to acknowledge the stream-as-truth interaction.

### Hard-won gotchas from Step 1 (memorize these)

**1. Durable-streams producer-id dedup trap.**

`emit_*_persisted` helpers in `crates/fireline-conductor/src/trace.rs` each call `stream.producer(format!(...)).build()` fresh per call. The SDK resets `next_seq` to 0 on every `.build()`, and server-side dedup is keyed on `(producer_id, epoch, seq)`. With the same producer_id across call sites, **every emit after the first comes back as `AppendReceipt { duplicate: true }` and is silently dropped.**

The smoking gun is the `duplicate: bool` flag in the durable-streams Rust SDK's `AppendReceipt`. It's not a body hash — it's a tuple check.

I only fixed `runtime_endpoints_persisted` (commit `5a07990`) by appending a `Uuid::new_v4()` suffix to the producer id. **`emit_runtime_spec_persisted` still uses a reused producer id** — it's currently safe because spec is monotonic (every call writes the same body, so dedup is "correct"), but it's fragile. If anyone adds a path that rewrites the spec with a different body, it'll silently vanish. Consider unifying the fix across both helpers, or moving to long-lived per-runtime_key producers (option D from the dispatch discussion).

**2. Parallel agent diagnosis beats solo theory.**

I spent ~30 minutes theorizing a concurrency ordering race between `register()` and `stop()` emits. A parallel agent (spotted via cmux) nailed the dedup trap in minutes by following the SDK code path to `AppendReceipt`. Lesson: when a stream observation is missing, check the SDK's dedup/fencing semantics *before* chasing timing races.

**3. Local tests passed, CI caught it.**

The dedup trap passed locally every time for me because my macbook executed emits fast enough that the first one was always the Ready state from register. CI's slower timing + different emit ordering surfaced the bug. **This is a case for the agreement test being CI-gated.** If it had only been a local test, the bug would have shipped.

**4. Shared-terminal 409 race is fixed.**

`sandbox_stop_and_recreate_preserves_session_load` was failing with a SharedTerminal `Busy` 409 early in the session. workspace:9 fixed it in `4cc6c75` with a 10×10ms grace-period retry loop in `src/routes/acp.rs::attach_terminal_with_grace_period`. If you see 409 "runtime_busy" in new tests, the retry is already there — the issue is elsewhere.

## Runtime-abstraction push — next phase direction

gnijor's goal: **push on the runtime abstraction** in the next phase. Currently there are two `RuntimeProvider` implementations:

- `LocalProvider` (`crates/fireline-control-plane/src/local_provider.rs`) — spawns the `fireline` binary as a host subprocess
- `DockerProvider` (`crates/fireline-conductor/src/runtime/docker.rs`) — builds a Docker image and runs the runtime in a container

gnijor is **evaluating `superradcompany/microsandbox`** (GitHub) as a candidate third provider. The user hasn't chosen yet — they said "is something im evaluating now." Do not assume adoption.

### Where microsandbox would slot in

A microsandbox runtime provider would sit alongside `LocalProvider` and `DockerProvider` in the `RuntimeProvider` trait (`crates/fireline-conductor/src/runtime/provider.rs`). The trait requires:

- Async `start(spec: CreateRuntimeSpec, runtime_key, node_id, mounted_resources) -> RuntimeLaunch`
- A `Box<dyn ManagedRuntime>` with `async fn shutdown()`

microsandbox's value proposition (VM-backed sandboxing, faster-than-Docker startup, stronger isolation than `LocalProvider`) would make it a natural fit for the "Sandbox" primitive in the managed-agent mapping. It could also replace `DockerProvider` for the shell-visible-mount invariant if its filesystem semantics are container-compatible.

### Things to verify before committing to microsandbox

1. **Does it provide a `MountedResource`-equivalent bind-mount surface?** `crates/fireline-conductor/src/runtime/mounter.rs::MountedResource { host_path, mount_path, read_only }` is the internal contract. If microsandbox can honor it (mount host paths into the sandbox fs), it plugs in directly.
2. **Does the guest inside the microsandbox have network access back to the host's durable-streams server?** Push-mode depends on the child POSTing `register` to the control plane and writing to a shared state stream. If microsandbox isolates network, we need a way to punch it through.
3. **How does it handle the `fireline` binary itself?** Docker builds the binary inside the image via the Dockerfile. microsandbox might expect a pre-built binary or might want to build it during the sandbox launch. The current `DockerProvider` uses `--docker-build-context` + `--dockerfile`; microsandbox will need analogous plumbing.
4. **Does it have an in-process harness suitable for the existing `control_plane_docker.rs`-style integration test?** If so, the same test shape can prove cross-provider equivalence (the invariant `sandbox_cross_provider_behavioral_equivalence` currently cross-references).

### Suggested first cut (if microsandbox is chosen)

- **Commit A:** Add a `MicrosandboxProvider` trait impl mirroring `DockerProvider`'s shape. Stub the actual microsandbox interaction with a clearly-marked `todo!()` until the dependency is wired up.
- **Commit B:** Add a `crates/fireline-conductor/src/runtime/microsandbox.rs` that implements the provider against a real microsandbox dependency. Corresponding `RuntimeProviderKind::Microsandbox` variant.
- **Commit C:** Add a `tests/control_plane_microsandbox.rs` integration test structured like `control_plane_docker.rs`, runtime-gated on microsandbox availability.
- **Commit D:** If behavior is equivalent to Docker, promote `sandbox_cross_provider_behavioral_equivalence` in `tests/managed_agent_sandbox.rs` to be a live test that runs both Docker and microsandbox runtimes against the same shared stream.

Scope estimate: 2-4 days of focused work, depending on how much of microsandbox's API matches the `RuntimeProvider` contract out of the box.

## Full-featured demo — what's still missing

The demo scoreboard at 32 passing tests proves the substrate. A "full featured demo" is a different bar: **end-to-end flow a human can watch and believe.** What's still missing:

### End-to-end flow gaps

1. **Live tool dispatch for `TransportRef::McpUrl`.** Slice 17 (commit `2681f50` from the earlier session) emits `tool_descriptor` envelopes but does NOT actually connect to external MCP URLs or forward tool calls. A demo showing an agent using a real remote MCP tool (e.g., Notion via Smithery) needs this. Medium scope (~1 day). Follow-up to slice 17.
2. **`resume(sessionId)` end-to-end in TS.** workspace:10 shipped the helper in `36096b7` with vitest coverage against a real control plane. A demo-quality story would wrap it in a UI: start a session, pause at approval, kill the runtime, resume elsewhere, see the state come back. The substrate proves all of that in `harness_durable_suspend_resume_round_trip` (`tests/managed_agent_harness.rs`) but there's no UI exercising it.
3. **A minimal UI.** None exists. The `packages/client` is headless. A demo would benefit from even a 500-line chat UI that shows: prompt → streaming response → approval gate → resume across restart. Scope depends on framing — probably Next.js in `packages/browser-harness` where the skeleton already exists.
4. **Resources launch-spec round-trip in TS.** The `resources` field landed in `4eaf94a` but nothing in TS actually constructs one in a real demo flow. A "here's how you launch an agent with a mounted workspace" path needs to exist.

### Substrate gaps surfaced by the agreement test

1. **`runtime_spec_persisted` producer-id dedup trap** (see §"Hard-won gotchas" above). Not a demo blocker, but fragile.
2. **Step 2 of stream-as-truth is unfinished.** The production control plane is still write-through to the registry. Not demo-blocking because the registry + the stream agree, but it's the difference between "stream-as-truth-in-theory" and "stream-as-truth-in-production."
3. **`RuntimeHost` split proposal is a document, not code.** The split is the right next structural cut after stream-as-truth Step 3 lands.

## Concrete next-move list (ranked)

Pick in order unless you have a reason to jump.

### A. Verify the topology hasn't drifted since commit

Run `git status`, `git log --oneline -5`, `gh run list --limit 3`. Confirm `0af8612` is HEAD on main and CI is green. If gnijor landed commits in another session, read them first.

### B. Runtime-abstraction push — microsandbox evaluation

1. **Read `https://github.com/superradcompany/microsandbox` README.** Verify the four questions in §"Things to verify before committing to microsandbox" above. Answer them concretely in an exploration doc at `docs/explorations/microsandbox-evaluation.md`. Don't commit production code yet.
2. **If evaluation is positive**, propose a 3-4 commit sequence (Commits A–D above) as a doc `docs/proposals/microsandbox-runtime-provider.md` mirroring the shape of `docs/proposals/runtime-host-split.md`. Get gnijor's sign-off before implementing.
3. **If negative**, document the findings (what's missing, what's incompatible) and propose either another provider or a path to fix microsandbox's gaps.

### C. Demo-readiness — pick one gap to close

- **Lowest-cost:** a short `docs/demo-script.md` that walks through what to show (create runtime, watch state stream, trigger approval, resume across restart) and which commands to run. Zero code.
- **Medium-cost:** live tool dispatch for `TransportRef::McpUrl`. Connect to an existing Smithery-hosted MCP via rmcp's `StreamableHttpClientTransport`; forward tool calls. Enables "watch the agent call a real external tool" in the demo.
- **Biggest bang-for-buck:** a minimal Next.js UI in `packages/browser-harness` that exercises the existing `@fireline/client` API. Shows streaming, approval gate, and resume. Several days of work but produces the thing a human actually watches.

### D. Finish stream-as-truth Step 2 (production read-path flip)

Only if there's a reason to push this before the demo. Adds shared-state subscription to the production control plane and flips read handlers to use `RuntimeIndex`. Small-ish (maybe half a day) but touches code in two crates and requires a new CLI flag + docs update.

### E. Fix the `runtime_spec_persisted` producer-id dedup trap

Currently safe in practice but fragile. Unify the UUID-suffix fix across both `emit_runtime_spec_persisted` and `emit_runtime_endpoints_persisted`, or move to long-lived single-producer-per-runtime_key. ~30 lines in `crates/fireline-conductor/src/trace.rs`.

### F. `RuntimeHost` split execution (commits 1–5 of the proposal)

Explicitly deferred to after stream-as-truth Steps 2/3. If you're doing Step 3, the `RuntimeHost` split's `RuntimeLifecycle` becomes largely redundant; re-read `docs/proposals/runtime-host-split.md` §6 for the interaction notes before starting.

## Agent coordination state

At handoff time, the `cmux` topology was:

- **workspace:4** — me (the lead coordinator). Idle at this doc.
- **workspace:10** — Claude Code, ~7-8h of context used. Shipped `36096b7` (TS resume), `54f4364` (runtime-host-split proposal), `3244eb7` (stream-as-truth interaction update), `d3d7e7f` (arch doc). Well-calibrated for TS + docs + cross-cutting lanes. Still online as of handoff.
- **workspace:8** — standing down at 21% ctx after shipping `5729584`, `cbf49a1`, `02363e6`, `053209b` (all doc-hygiene lanes). Fine to leave idle or re-spawn.
- **workspaces 6, 7, 9** — closed at some point in the session. workspace:9 had shipped `a4dc19c` (harness durable suspend/resume), `4cc6c75` (SharedTerminal 409 fix), `6045c4a` (Item 3 liveness unify) before closing. workspace:6 had shipped `b82f642`, `91bbb0b`, `8cc07ed` (semantic kernel + formal invariants) before closing.

If you're spinning up new agents for the next phase, workspace:10 (Claude Code) is proven good at cross-cutting work and handled the runtime-host-split proposal well. A new Rust-substrate-heavy agent for microsandbox work is the most obvious fresh lane.

## Open decisions that need gnijor

These are the things that surfaced this session that don't have answers yet:

1. **microsandbox adoption.** Gnijor said "evaluating." Don't commit production dependency on it without explicit approval.
2. **Stream-as-truth Step 2 timing.** Demo-safe to defer; architecturally valuable to land. Gnijor's call.
3. **Docker CI step re-enable.** Deferred in `0af8612`. Re-enabling requires pre-built image + layer caching. Gnijor's call on priority.
4. **`RuntimeHost` split vs. direct stream-as-truth Step 3.** The split proposal has a 5-commit transition plan. Step 3 of stream-as-truth makes parts of the split redundant. Gnijor's call on sequence — but stream-as-truth first is the right order IMO.
5. **Fate of `verification/` and `crates/fireline-semantics/`.** workspace:6's formal layer is a net win architecturally but it's a separate crate with its own testing story. No decision needed unless you're deleting/consolidating.

## Files that changed a lot this session (read them to understand the current state)

- `crates/fireline-conductor/src/trace.rs` — `emit_runtime_endpoints_persisted`, shutdown-flush fix, UUID fix
- `crates/fireline-conductor/src/runtime/mod.rs` — four emit sites for `runtime_endpoints`
- `src/runtime_index.rs` — new projection (3 maps: specs, instances, endpoints)
- `src/bootstrap.rs` — direct-host spec emit + shutdown flush
- `src/lib.rs` — visibility tightening
- `src/routes/acp.rs` — SharedTerminal grace-period retry (from workspace:9)
- `tests/runtime_index_agreement.rs` — the agreement invariant integration tests
- `tests/managed_agent_harness.rs` — D test promoted (cross-runtime suspend/resume round-trip)
- `packages/client/src/host.ts` — `resources` field + `resume(sessionId)` helper
- `docs/architecture.md` — replaced with the comprehensive arch2 synthesis
- `docs/proposals/runtime-host-split.md` — the post-stream-as-truth proposal
- `verification/stateright/src/lib.rs` + `crates/fireline-semantics/src/stream_truth.rs` — formal layer for runtime_index agreement

## Closing notes

This session's strongest architectural commitment: **the durable stream is the source of truth, not an in-memory cache.** Every cut this session reinforced that thesis — the `RuntimeIndex` projection, the agreement invariant test, the formal Stateright check, the runtime-host-split proposal's stream-as-truth update. The next phase should continue the same trajectory: if something about a runtime's existence, lifecycle, or identity needs to survive restart or be observable cross-runtime, **it belongs in the stream**.

The microsandbox evaluation is a chance to prove the `RuntimeProvider` abstraction is really abstract. Today it has two implementations (Local, Docker) that are deeply similar. A third implementation that's genuinely different (microsandbox VM semantics vs. Docker container semantics) is the forcing function that will surface any hidden coupling.

The demo-readiness question is separate. Substrate is solid; end-to-end human-observable flows are where the gap is. Picking ONE demo-specific gap to close (C in the ranked list) is probably higher leverage than another architectural refactor for a demo-watching human.

Good luck.
