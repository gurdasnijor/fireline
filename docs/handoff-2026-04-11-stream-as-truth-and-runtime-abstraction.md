# Session Handoff — Stream-As-Truth Phase 1 & Runtime Abstraction Push

> **Created:** 2026-04-11 (late morning, immediately after the demo-day debt-paydown session)
> **Author:** Claude Opus 4.6 session (workspace:4, lead coordinator)
> **For:** the next session continuing the runtime-abstraction push and driving toward a full-featured demo
> **Companion handoffs:** `docs/handoff-2026-04-11-managed-agent-suite.md` (original demo-day starting point) and `docs/handoff-2026-04-11-post-debt-paydown.md` (post-arch-debt-paydown state)

This is the third handoff in the day's sequence. Read those two first for the full arc; this one focuses on what landed *after* the post-debt-paydown point and on the two directions gnijor named for the next phase: **pushing on the runtime abstraction** and **getting closer to a full-featured demo** — with **`superradcompany/microsandbox`** currently under evaluation as a candidate provider.

## Current in-flight work (as of 2026-04-11)

- **Crate restructure** — dispatched (codex workspace:13); manifest at `c9a3e8e`
- **TLA Level 2** — dispatched (codex workspace:12)
- **Tier 4 host-claude** — landed `2348428`, then **deleted in `37db346`** along with the rest of the Claude-host satisfier code; design lessons preserved in `docs/explorations/claude-agent-sdk-v2-findings.md` (retained as thought-experiment history)
- **Host primitive rename** — landed `37db346`: `createSession → provision`, `SessionHandle → HostHandle` (now carries `acp` + `state` endpoints), `SessionSpec → ProvisionSpec`, `SessionStatus → HostStatus`, `stopSession → stop`, `sendInput` / `SessionInput` / `SessionOutput` deleted; `packages/client/src/host-claude/` removed. Doc sync across the client and deployment proposals, `runtime-host-split.md`, and the findings doc above
- **Tier 5 browser demo** — was in progress on workspace:4 at the time of this handoff

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
3. **A minimal UI.** None exists. The `packages/client` is headless. A demo would benefit from even a 500-line chat UI that shows: prompt → streaming response → approval gate → resume across restart. Scope depends on framing — likely as a dedicated example app under `examples/`.
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
- **Biggest bang-for-buck:** a minimal example UI that exercises the existing `@fireline/client` API. Shows streaming, approval gate, and resume. Several days of work but produces the thing a human actually watches.

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

## Verification-layer follow-ups (logged mid-session, 2026-04-11 afternoon)

These items were scoped during the Host/Sandbox/Orchestrator primitive-split dispatch and deferred behind the current code lanes. They are demo-safe to skip and architecturally load-bearing to eventually land. Pick them up after the current code lanes (Rust Tier C-D, TS Tier 1-2) stabilize.

### Background

The verification layer has three physical locations today:

- **`crates/fireline-semantics/`** — pure Rust kernel crate (workspace member, zero deps). Encodes protocol semantics as pure functions (session, resume, approval, liveness, stream-truth modules).
- **`verification/stateright/`** — `fireline-verification` crate (workspace member). `depends on fireline-semantics.workspace = true`. Wraps the kernel in a Stateright bounded model check.
- **`verification/spec/managed_agents.tla`** — TLA+ spec (Java TLC, independent second formalization).
- **`verification/docs/refinement-matrix.md`** — maps TLA+ invariants → Rust kernel functions → executable tests.
- **`verification/README.md`** — explains the three layers.

The dependency direction is correct: `verification/stateright` → `crates/fireline-semantics` → nothing. `fireline-semantics` is a pure library that could eventually be consumed by production code for runtime invariant assertions, not just verification, which is why it lives in `crates/` and not under `verification/`. Physical co-location was considered and rejected: the split is architecturally meaningful (kernel vs. verification-specific harness) and shouldn't be collapsed.

### TLA+ spec drift — three levels of catch-up

After the Host/Sandbox/Orchestrator reframe in `docs/proposals/client-primitives.md` + `docs/proposals/runtime-host-split.md` §7, the TLA+ spec at `verification/spec/managed_agents.tla` encodes the old vocabulary. It still models 6 of the 7 Anthropic primitives correctly (Session, Orchestration, Harness, Sandbox, Resources, Tools) and its invariants are all valid — but its action names and variable names don't match the new primitive split. Three levels of update, in order of cost:

**Level 1 — vocab rename + one new invariant (~40-line diff, ~30-45 min):**
- Top-of-file comments pointing at `docs/proposals/client-primitives.md` and `docs/proposals/runtime-host-split.md` §7 as the authoritative primitive taxonomy the spec is tracking.
- Collapse `ResumeLive` + `ResumeCold` into a single `Wake` action with an internal branch on `beforeStatus = "ready"` vs `"stopped"`. Rename `lastResume` → `lastWake`, `resumeEpoch` → `wakeEpoch`, `resumeResponses` → `wakeResponses`.
- Rename the existing invariants: `ResumeOnLiveRuntimeIsNoop` → `WakeOnReadyIsNoop`, `ColdResumePreservesRuntimeKeyChangesRuntimeId` → `WakeOnStoppedChangesRuntimeId`.
- Add inline comments on `runtimeIndex`, `mountedResources`, and `ProvisionRuntime` tagging each as "Host state" / "Sandbox state" / "Host-Sandbox composition" so a future reader can see where the split would land.
- Add one new named invariant: `WakeIsHostOnly` — all state transitions on `runtimeIndex[rk].status` come from `Wake`, `StopRuntime`, or `ProvisionRuntime`, never from a Harness action or an approval action. This is already true in the spec; we just give it a name.
- Run TLC locally once to verify the renamed spec is still green. Java TLC is ~2 min; single run is fine (not a cargo run, not subject to the battery constraint that was applied to the Rust test suite).
- Zero semantic change. Preserves every existing invariant. No risk to demo.

**Level 2 — new invariants the reframe makes formally statable (~80-line diff, post-demo):**
- `HostDoesNotReachIntoSandboxTools`: approval/suspend combinators never mutate `mountedResources` or `toolRegistry`.
- `SandboxIndependentOfOrchestration`: wake episodes don't mutate `mountedResources` except via the explicit `newRid → requestedResources[rk]` propagation.
- `CombinatorAlgebraIsClosed`: every `HarnessEmit` kind is one of the 7 combinator outputs (today it's implicit; name it).
- Refine the `Sandbox` model to distinguish "runtime can run tools" from "runtime is reachable for ACP" — one is a reachability property, the other is a Host concern.

**Level 3 — structural refactor for the full Host/Sandbox/Combinator split (200+ line diff, post-demo):**
- Split `runtimeIndex` into `hostState` + `sandboxState` as separate variables.
- Split `ProvisionRuntime` into `CreateSession` + `ProvisionSandbox` actions.
- Add an explicit `Combinator` record type with 7 variants and a `Topology == Seq[Combinator]` per session.
- Add `ApplyCombinator(s, c, e)` that dispatches on combinator kind and shows how each kind transforms the session log.
- Verify the existing invariants survive the refactor (most should — the primitives are the same, only the decomposition changes).

Level 3 is real formal-modeling work. It is the right post-demo target to make the TLA+ spec match the TS `client-primitives.md` doc 1:1, but it is a multi-hour focused slice.

### CI + drift-prevention hygiene (~15-min commit, independently landable)

Today the managed-agent-suite workflow (`.github/workflows/managed-agent-suite.yml`) runs the Rust integration suite. It does NOT run `crates/fireline-semantics` unit tests, does NOT run `verification/stateright` Stateright checks, and does NOT run TLC on the `.tla` spec. All three verification artifacts can silently drift from the production code they're supposed to track.

Three additive items, one commit:

1. **CI coverage end-to-end.** Add `fireline-semantics` unit tests + `fireline-verification` Stateright checks to a new `verification-layer` workflow filtered on `crates/fireline-semantics/**` + `verification/**`. Optional: add a TLC step using `setup-java@v4` + `wget tla2tools.jar` + `java -jar tla2tools.jar -config ...` (~15 lines of YAML).

2. **Refinement matrix cross-references.** Read `verification/docs/refinement-matrix.md` and verify that every TLA+ invariant name cross-references a specific `fireline-semantics` function/type name AND a specific `tests/managed_agent_*.rs` test function. If any are missing, add them. Keeps the three formalizations in traceable lockstep.

3. **Drift-prevention rule in README.** Add one sentence to `verification/README.md`: *"Any change to managed-agent primitive semantics in `crates/fireline-conductor/src/runtime/**`, `crates/fireline-conductor/src/primitives/**`, or `crates/fireline-control-plane/**` must be reflected in the same commit (or an immediately-following commit) in `crates/fireline-semantics/src/**`, `verification/stateright/src/**`, and `verification/spec/managed_agents.tla`."*

### Physical unification — explicitly rejected as of this session, revisitable post-demo

Considered and rejected: moving `crates/fireline-semantics/` into `verification/semantics/` to co-locate all verification artifacts. Reasoning for keeping the split: `fireline-semantics` is a pure library that could be consumed by production code for runtime invariant assertions, not just verification. Co-locating it with verification-specific artifacts would couple "reusable kernel" to "verification-only harnesses." The current dependency graph (`fireline-verification` → `fireline-semantics`) is correct and standard Rust workspace layout.

If the drift-prevention hygiene items above still fail to prevent silent slippage after a few sessions of practice, revisit this as a post-demo cleanup. The rename would be ~30 lines (Cargo manifest path + one workspace member entry + a few imports). Not urgent.

### Estimated total cost

- Level 1 TLA+ update: 30-45 min
- CI + drift-prevention hygiene: 15 min
- Level 2 TLA+ update: 1-2 hours (post-demo)
- Level 3 TLA+ structural refactor: half-day to full day (post-demo)
- Physical unification: ~30 min if ever needed (post-demo)

All items are independently landable in any order. None of them block the current code lanes (Rust primitive split, TS client core, demo UI rewrite).

## Crate restructure follow-up (logged mid-session, 2026-04-11 afternoon)

The current Rust crate layout (`fireline-conductor`, `fireline-components`, `fireline-control-plane`, `fireline-semantics`, plus the `fireline` binary at `src/`) is a slice-by-slice accumulation that doesn't map cleanly to the Anthropic six-primitive taxonomy. Each of the current crates crosses 3-4 primitive boundaries. The target layout is one crate per primitive, primitive-aligned at the crate level:

```
crates/
├── fireline-session/       — Session primitive: SessionLog interface + stream-backed satisfier, index projections
├── fireline-orchestration/ — Orchestration primitive: Orchestrator/WakeHandler + whileLoop/cron/http satisfiers
├── fireline-harness/       — Harness primitive: conductor proxy chain + combinator interpreter
├── fireline-sandbox/       — Sandbox primitive: tool-execution satisfiers (local subprocess, docker, microsandbox)
├── fireline-resources/     — Resources primitive: ResourceMounter + file backends
├── fireline-tools/         — Tools primitive: ToolDescriptor + CapabilityRef + TransportRef + attach_tool/smithery/peer bridges
├── fireline-semantics/     — pure formal kernel (unchanged)
├── fireline-control-plane/ — HTTP deployment assembly (wraps Session + Orchestration + Host primitives)
└── fireline-runtime/       — the `fireline` binary assembly (wraps Harness + ACP transport)
```

**9 crates total.** Each crate has ONE reason to change — its primitive. New satisfier implementations slot in obviously: a new Session backend goes in `fireline-session`, a new Sandbox impl in `fireline-sandbox`, and so on. The crate name IS documentation for what the crate does. This is the biggest migration but produces the cleanest long-term architecture: symmetric with the TS-side `@fireline/client/core` per-module split, with every crate boundary serving a real separation-of-concerns purpose.

An alternative "consolidated" shape (one `fireline-substrate` crate with 6 primitive modules, 4 crates total) was considered and rejected as insufficiently separated: it would collapse the per-primitive boundaries back into one large crate and undo the clarity of the split. The bigger migration is the right trade.

### Why post-demo and not now

This is a ~1-1.5 day migration, touches every crate, breaks imports across the repo, requires a full CI pass to verify. Roughly 40-50 file moves + 6 new crates + ~500-700 lines of use-path updates across downstream crates/tests. Doing it now would require freezing all other code lanes. Demo comes first.

### Partial realization already shipped

workspace:10's `crates/fireline-conductor/src/primitives/{host,sandbox,orchestration}.rs` (from commit c794e35, Tier C) is a small partial realization of the target shape. When the migration happens, those three files map cleanly into the new crates:

- `crates/fireline-conductor/src/primitives/host.rs` → contents move to the `host` module inside the new `fireline-harness` crate (since Host is really the Harness primitive's session-lifecycle concern — the Anthropic primitive table keeps Session and Harness separate, and our internal Host interface lives where the conductor's runtime-lifecycle code lives today, which is Harness territory)
- `crates/fireline-conductor/src/primitives/sandbox.rs` → `fireline-sandbox/src/traits.rs`
- `crates/fireline-conductor/src/primitives/orchestration.rs` → `fireline-orchestration/src/traits.rs`

No work from Tier C is wasted.

### File-by-file migration map (reference for when execution happens)

```
crates/fireline-conductor/src/proxy.rs              → fireline-harness/src/proxy.rs
crates/fireline-conductor/src/state_projector.rs    → fireline-session/src/state_projector.rs
crates/fireline-conductor/src/trace.rs              → fireline-session/src/trace.rs
crates/fireline-conductor/src/runtime/**            → fireline-harness/src/runtime/**     (runtime-lifecycle = Harness internals)
crates/fireline-conductor/src/primitives/host.rs    → fireline-harness/src/host.rs        (merge with runtime/)
crates/fireline-conductor/src/primitives/sandbox.rs → fireline-sandbox/src/traits.rs
crates/fireline-conductor/src/primitives/orchestration.rs → fireline-orchestration/src/traits.rs
crates/fireline-conductor/src/topology.rs           → fireline-harness/src/topology.rs
crates/fireline-conductor/src/shared_terminal.rs    → fireline-harness/src/shared_terminal.rs
crates/fireline-conductor/src/transports/**         → fireline-runtime/src/transports/**   (deployment concern)

crates/fireline-components/src/approval.rs          → fireline-harness/src/combinators/approval.rs
crates/fireline-components/src/budget.rs            → fireline-harness/src/combinators/budget.rs
crates/fireline-components/src/context.rs           → fireline-harness/src/combinators/context.rs
crates/fireline-components/src/fs_backend.rs        → fireline-resources/src/fs_backend.rs
crates/fireline-components/src/peer/**              → fireline-tools/src/peer/**
crates/fireline-components/src/smithery.rs          → fireline-tools/src/smithery.rs
crates/fireline-components/src/attach_tool.rs       → fireline-tools/src/attach_tool.rs
crates/fireline-components/src/tools.rs             → fireline-tools/src/descriptor.rs
crates/fireline-components/src/sandbox/microsandbox.rs → fireline-sandbox/src/microsandbox.rs  (stays feature-gated behind `microsandbox-provider`)

src/bootstrap.rs                                    → fireline-runtime/src/bootstrap.rs
src/routes/**                                       → fireline-runtime/src/routes/**
src/runtime_host.rs                                 → fireline-runtime/src/runtime_host.rs
src/orchestration.rs                                → fireline-orchestration/src/fireline_host.rs
src/runtime_index.rs                                → fireline-harness/src/runtime_index.rs
src/session_index.rs                                → fireline-session/src/index.rs
src/runtime_materializer.rs                         → fireline-session/src/materializer.rs
src/stream_host.rs                                  → fireline-runtime/src/stream_host.rs
src/main.rs                                         → fireline-runtime/src/main.rs
src/bin/**                                          → fireline-runtime/src/bin/**
```

### Dependency graph (target)

Each primitive crate has one clear reason to change and a narrow downward dep set:

```
fireline-session ─────────────┐
fireline-resources ───────────┤
fireline-tools ───────────────┤→ no downstream deps within substrate
                              │
fireline-sandbox ─────────────┤→ depends on: (none)
                              │
fireline-harness ─────────────┤→ depends on: fireline-session, fireline-tools, fireline-sandbox, fireline-resources
                              │   (the conductor chain needs to compose combinators that touch all of them)
                              │
fireline-orchestration ───────┘→ depends on: fireline-session (for the SessionRegistry + materialized state read)
                                  NOTE: does NOT depend on fireline-harness. Orchestrator only knows about wake(session_id).

fireline-semantics            → depends on: (none — pure formal kernel)
                                  Consumed by: fireline-verification (in verification/stateright/)

fireline-control-plane        → depends on: fireline-harness (for Host trait + runtime lifecycle),
                                             fireline-session (for state-stream writer),
                                             fireline-orchestration (for wake endpoint)

fireline-runtime (binary)     → depends on: fireline-harness (conductor chain),
                                             fireline-session (trace writer),
                                             fireline-orchestration (wake handler),
                                             fireline-control-plane (optional push-mode client)
```

Critical property: `fireline-orchestration` does NOT depend on `fireline-harness`. That's the load-bearing separation the Anthropic primitive table names — orchestration is scheduler-only, harness is execution-only, and they compose through the `wake(session_id) → void` contract. If a future satisfier ships an orchestrator that doesn't use the Fireline harness (e.g., a cron orchestrator driving a Claude-managed host), it just depends on `fireline-orchestration` with no harness pullthrough.

### Transition plan (when executed)

1. Create empty `crates/fireline-session/`, `fireline-orchestration/`, `fireline-harness/`, `fireline-sandbox/`, `fireline-resources/`, `fireline-tools/` with `Cargo.toml` + `lib.rs` + empty module structure (6 new crates)
2. Create empty `crates/fireline-runtime/` with `Cargo.toml` + `main.rs` stub (1 new crate)
3. Update the workspace root `Cargo.toml` to list all 7 new crates as members
4. Per primitive (session, orchestration, harness, sandbox, resources, tools): `git mv` the relevant files from `crates/fireline-conductor/src/**` and `crates/fireline-components/src/**` into the new crate + fix up `mod.rs` declarations + add the new crate's minimal `Cargo.toml` deps. One commit per primitive, 6 commits total. Verify CI green after each.
5. Move `src/**` binary code into `crates/fireline-runtime/src/**` in one commit
6. Delete `crates/fireline-conductor/` and `crates/fireline-components/` (now empty shells) + remove from workspace members
7. Add `pub use` re-exports at each new crate's `lib.rs` for backward compat so downstream imports don't break immediately (e.g. in `fireline-harness/lib.rs`: `pub use runtime::*;` so `use fireline_harness::runtime_host::*` still works during the transition window)
8. Update downstream imports in `fireline-control-plane`, `fireline-verification`, and tests to use the new paths. This is the largest diff — probably ~500-700 lines, but mostly mechanical find-and-replace from `fireline_conductor::X` to `fireline_harness::X` or `fireline_session::X` depending on which primitive X belongs to.
9. Delete the backward-compat re-exports from step 7 in a follow-up commit once all downstream imports are clean
10. Bonus cleanup: verify the new dep graph (§ above) is actually enforced — no accidental back-edges from `fireline-orchestration` to `fireline-harness`, etc. If there are, extract the necessary types into `fireline-session` (the shared read surface) or into a new tiny `fireline-primitives-core` crate for the pure types.

Estimated cost: ~1-1.5 day of migration, half day of CI babysitting. Single focused slice, dedicated handoff doc, zero other lane work during execution.

## Closing notes

This session's strongest architectural commitment: **the durable stream is the source of truth, not an in-memory cache.** Every cut this session reinforced that thesis — the `RuntimeIndex` projection, the agreement invariant test, the formal Stateright check, the runtime-host-split proposal's stream-as-truth update. The next phase should continue the same trajectory: if something about a runtime's existence, lifecycle, or identity needs to survive restart or be observable cross-runtime, **it belongs in the stream**.

The microsandbox evaluation is a chance to prove the `RuntimeProvider` abstraction is really abstract. Today it has two implementations (Local, Docker) that are deeply similar. A third implementation that's genuinely different (microsandbox VM semantics vs. Docker container semantics) is the forcing function that will surface any hidden coupling.

The demo-readiness question is separate. Substrate is solid; end-to-end human-observable flows are where the gap is. Picking ONE demo-specific gap to close (C in the ranked list) is probably higher leverage than another architectural refactor for a demo-watching human.

Good luck.
