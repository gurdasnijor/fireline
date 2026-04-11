# Session Handoff — Managed-Agent Suite Push

> **Created:** 2026-04-11 (late night)
> **Author:** Claude Opus 4.6 session, managing agent swarm for @gnijor
> **For:** the next session continuing this work
> **Context usage at handoff:** 558.8k/1m (56%)
> **Demo deadline:** tomorrow (2026-04-11)

---

## Your role

You are the lead engineer **and** the manager of a swarm of parallel agents working on the Fireline Rust repo at `/Users/gnijor/gurdasnijor/fireline`. Gnijor is the human. He dispatches some agents directly (OTel instrumentation lane, orchestration-regression-fix lane); you dispatch others via the `Agent` tool for focused substrate work. Your job is to:

1. Keep the managed-agent test suite moving toward fully green.
2. Make architectural changes that align Fireline with its "substrate, not product" vision (see `docs/explorations/managed-agents-mapping.md` and `docs/explorations/typescript-typed-functional-core-api.md`).
3. Avoid collisions between parallel agent lanes — each lane owns specific files, and you should not touch files another lane is mid-flight on.
4. Commit in clean, reviewable logical chunks. Do not mix concerns. Attribute honestly when multiple lanes' changes are co-located in one file.
5. Before committing anything, **verify it actually passes** — do not ship a weakened oracle to make a test pass.

## TL;DR — repo state right now

```
HEAD: 2681f50 Add slice 17 capability profiles and attach_tool topology component
```

**Managed-agent suite: 27 live / 4 pending.** Started this session at 12 live / 18 pending. More than doubled. Session (5/5), Orchestration (3/3), and Tools (2/2) are fully green. The remaining 4 pending tests are either:
- **Docker-scoped** (2 tests, already covered end-to-end by `tests/control_plane_docker.rs`)
- **By-design cross-reference marker** (1 test, intentional)
- **Genuine substrate gap** (1 test: `harness_durable_suspend_resume_round_trip`, needs cross-runtime client shim)

Only ONE honest genuine gap remains: cross-runtime suspend/resume client shim. Everything else is either duplicated elsewhere, by design, or landed.

## Working tree at handoff

Four files uncommitted, all belonging to the **OTel instrumentation lane** (gnijor's other agent, working on perf troubleshooting):

```
 M crates/fireline-conductor/src/runtime/mod.rs
 M crates/fireline-control-plane/src/local_provider.rs
 M crates/fireline-control-plane/src/main.rs
 M src/main.rs
```

**Do not commit these.** They belong to the OTel agent's logical unit. When that agent reports done, it will commit its own work. If you see these files changing while you work, that's the OTel agent progressing.

Some OTel annotations ALREADY landed in `src/orchestration.rs` and `tests/managed_agent_primitives_suite.rs` (co-located with commits `e1bd331` and `2681f50`). Those are committed. The current uncommitted 4 files are the remaining OTel lane files.

## Session scoreboard (per primitive)

| Primitive | Live | Pending | Files |
|---|---|---|---|
| **Session** | **5** | **0** | `tests/managed_agent_session.rs` |
| **Sandbox** | 3 | 1 (by-design marker) | `tests/managed_agent_sandbox.rs` |
| **Harness** | 4 | 1 (real gap: cross-runtime shim) | `tests/managed_agent_harness.rs` |
| **Orchestration** | **3** | **0** | `tests/managed_agent_orchestration.rs` |
| **Resources** | 4 | 1 (Docker-only, covered elsewhere) | `tests/managed_agent_resources.rs` |
| **Tools** | **2** | **0** | `tests/managed_agent_tools.rs` |
| **Primitives suite** | 6 | 1 (Docker-only sibling of resources) | `tests/managed_agent_primitives_suite.rs` |

## Remaining 4 pending tests (detailed)

1. **`tests/managed_agent_harness.rs::harness_durable_suspend_resume_round_trip`** — the real gap. Needs a control-plane round-trip that can abandon a blocked prompt, re-provision a fresh runtime from the durable spec, and resume via a fresh `session/load`. The block-until-resolved half is already green (`harness_approval_gate_blocks_prompt_until_resolved_via_stream_event` commit 37ed1ec). What's missing is a test shim that can kill a runtime mid-prompt while preserving the original client's view long enough to observe a post-death `load_session` rebuild. The approval gate's `rebuild_from_log` helper exists; the missing piece is the cross-runtime client shim. This is substantial substrate work.

2. **`tests/managed_agent_sandbox.rs::sandbox_cross_provider_behavioral_equivalence`** — by-design marker. `#[ignore = "already covered end-to-end by tests/control_plane_docker.rs"]`. Do not promote. It's a cross-reference for primitive coverage visibility.

3. **`tests/managed_agent_resources.rs::resources_physical_mount_is_shell_visible_inside_runtime`** — Docker-scoped. Local runtimes run on the host with no container filesystem, so "shell sees the mount" has no analogue outside Docker. The honest treatment is to either delete this stub or update its pending message to say "covered by `tests/control_plane_docker.rs` slice 13c". Do not try to promote with a weakened oracle.

4. **`tests/managed_agent_primitives_suite.rs::managed_agent_resources_physical_mount_shell_visibility_contract`** — duplicate of #3 in the primitives-suite file. Same reasoning.

## Commits this session (newest first, 14 total)

```
2681f50 Add slice 17 capability profiles and attach_tool topology component
e1bd331 Replace orchestration shared-state discovery with explicit endpoint
7c92670 Promote session idempotent-append contract against PROTOCOL.md §5.2.1
76547ce Promote primitives-suite tools schema-only acceptance contract
b6156d9 Add pending runtime specs management to RuntimeHost   (gnijor's agent — fixed orchestration regression)
df83d9f Emit tool_descriptor envelopes and promote tools schema-only contract
ab1863a Promote session mid-offset replay contract
b0e7ef7 Add scripted fireline-testy-fs agent and promote two fs_backend tests
9f9122b Promote cross-runtime virtual-fs test and make Tools blockers honest
96f87ec Promote sandbox stop+recreate, session materialized agreement, and harness combinator coverage
af4d6a6 Promote live-runtime resume idempotency tests
37ed1ec Promote approval-gate blocking and subscriber-loop invariants
b3a86cc Align managed-agents-mapping status claims with live test coverage
7690dbb Make ApprovalGateComponent actually block on RequireApproval
59cc42c Make runtime materializer faithful to the State Protocol v1.0
d1e13df Stabilize managed-agent test harness and make testy_load durable
```

Every commit has a `Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>` trailer. Preserve that convention.

## Architectural changes landed this session (high-level)

Each of these is load-bearing for future work. Read the commit if you need to rationalize something.

### 1. Runtime materializer is State Protocol v1.0 compliant (`59cc42c`)

`src/runtime_materializer.rs` was rewritten to:
- Parse each event independently — a malformed event no longer poisons the whole chunk.
- Recognize control messages by `headers.control` — `snapshot-start`, `snapshot-end`, `reset`. `reset` calls `StateProjection::reset` on every projection, which is a new trait method with a default no-op and overrides on `SessionIndex` and `ActiveTurnIndex`.
- Only accept spec operations `insert`/`update`/`delete` on change events.
- Normalize `fs_backend` producers to emit `operation: "update"` instead of the Fireline-specific `"upsert"` extension — the whole codebase is now spec-compliant.

**Reference**: `packages/state/STATE-PROTOCOL.md` v1.0 in the upstream durable-streams repo.

**Gotcha**: this rewrite is slightly slower than the old single-parse-fail-whole-chunk path. It exposed a pre-existing race in `RuntimeHost::create` where the runtime_spec envelope could be skipped if the runtime registered before `emit_runtime_spec_persisted` was reached. That race was fixed separately by gnijor's agent in `b6156d9` (pending_runtime_specs map).

### 2. Approval gate actually blocks (`7690dbb`)

`crates/fireline-components/src/approval.rs` previously forwarded every prompt to the agent first, then emitted a permission_request. That was an observer wearing a gate costume. Now:
- On `RequireApproval`: emit `permission_request` with a uuid `request_id`, flush the producer, open a LiveMode::Sse reader on the stream, wait for an `approval_resolved` event with matching `requestId`, only then forward to the agent (or error out on denial).
- Added `approval_timeout: Option<Duration>` so tests can bound the wait.
- Default topology wiring no longer hardcodes `PromptContains { needle: "" }` (which matched every prompt). It takes a real `policies` field from the topology config; empty policies = no-op gate.

**Test machinery**: see `tests/support/managed_agent_suite.rs::append_approval_resolved` + `wait_for_permission_request` helpers. Two tests use this pattern: `harness_approval_gate_blocks_prompt_until_resolved_via_stream_event` and `orchestration_subscriber_loop_drives_pause_release_cycle`.

### 3. Orchestration shared-state endpoint is explicit (`e1bd331`)

`fireline::orchestration::resume` previously discovered the shared state stream URL by listing runtimes and picking the first non-empty `state.url`. That was a hack. Now the signature is:

```rust
pub async fn resume(
    http: &HttpClient,
    control_plane_url: &str,
    shared_state_url: &str,    // ← new, explicit
    session_id: &str,
) -> Result<RuntimeDescriptor>
```

The `shared_state_stream_url()` helper and its `list_runtimes` dependency are deleted. Callers pass the URL explicitly — matches the TS API doc's `resumeSession(control, shared_state, session_id)` shape. Added `ControlPlaneHarness::shared_state_url()` accessor so test callsites stay ergonomic.

**This is the vision-aligned version**: it makes the wire boundary honest.

### 4. Capability profiles / slice 17 (`2681f50`)

`crates/fireline-components/src/tools.rs` now has:
- `TransportRef` enum — `PeerRuntime`, `Smithery`, `McpUrl`, `InProcess`
- `CredentialRef` enum — `Env`, `Secret`, `OauthToken`
- `CapabilityRef { descriptor, transport_ref, credential_ref }`

**Serde gotcha** (LOAD-BEARING): the enums use `#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]`. The `rename_all_fields` is critical — without it, inner-variant fields like `runtime_key` serialize as snake_case, which breaks JSON deserialization from topology config. There's a regression guard unit test `transport_ref_field_names_are_camel_case_on_the_wire` in `crates/fireline-components/src/tools.rs`. Do not remove it.

New `crates/fireline-components/src/attach_tool.rs` defines `AttachToolComponent` that takes a `Vec<CapabilityRef>`, enforces first-attach-wins on tool-name collisions (pinned collision rule), emits `tool_descriptor` envelopes via the existing `emit_tool_descriptor` helper with source="attach_tool", and passes through as an sacp::Proxy. **Slice 17 is descriptor-emission + collision-rule only** — live dispatch via `TransportRef` (actually connecting to an external MCP URL, resolving credentials, forwarding tool calls) is deliberately out of scope. A follow-up slice can add the resolver.

`peer_mcp` and `smithery` registrations in `src/topology.rs` are NOT yet migrated to `attach_tool`. They still exist as bespoke factories. Migrating them is a logical follow-up but was deliberately deferred to keep slice 17 self-contained.

### 5. testy_load is durable (`d1e13df`)

`src/bin/testy_load.rs` used to keep session state in-memory only, so `session/load` after a runtime restart returned `session_not_found`. That was blocking the orchestration acceptance contract. Now testy_load reads `FIRELINE_ADVERTISED_STATE_STREAM_URL` from env and, on `session/load`, scans the durable stream for a matching session envelope. If it finds one, it rebuilds in-memory state and accepts. If the env var is unset (local bootstrap path), it falls back to the old in-memory-only behavior — that path is pinned by `tests/session_load_local.rs` so don't flip it.

### 6. Scripted `fireline-testy-fs` agent (`b0e7ef7`)

`src/bin/testy_fs.rs` is a new test-only agent that responds to JSON prompts of shape `{command: "write_file", path: "...", content: "..."}` by issuing an ACP `fs/write_text_file` request against its client connection. Also supports `ReadFile` and `Ready` commands. Used by resources tests to deterministically emit fs effects without relying on a real agent. Helper is `testy_fs_bin()` in `tests/support/managed_agent_suite.rs`.

## The vision you are serving

These are the docs that ground every architectural decision. Read or re-read them whenever you're uncertain what shape something should take.

- **`docs/explorations/managed-agents-mapping.md`** — Maps Fireline to Anthropic's six managed-agent primitives (Session, Orchestration, Harness, Sandbox, Resources, Tools). The "seven combinators" framing (observe, mapEffect, appendToSession, filter, substitute, suspend, fanout) is how Fireline composes over those primitives. Every status row in the table at the top should match reality — I brought it into alignment in `b3a86cc` with file:line citations, keep it honest.

- **`docs/explorations/typescript-typed-functional-core-api.md`** — Proposal for the TS substrate API. Core rule: **if the runtime executes it, model it as data; if the local TS process executes it, model it as a pure function.** Data-first at the wire boundary (`TopologySpec`, `ResourceRef`, `CapabilityRef`, `RuntimeSpec`), pure functions for local interpreters (materializers, middleware). This is the rule that made slice 17 obvious and that also made the orchestration shared-state cleanup obvious. When in doubt, ask: "does this code run in the Rust runtime, or does it run in the local caller process?" That tells you whether it's data or a function.

- **`PROTOCOL.md`** in the upstream durable-streams repo at `~/.cargo/git/checkouts/durable-streams-76b5902550dc7fbe/ef708da/` — the authoritative source for stream semantics. §5.2.1 has Kafka-style idempotent producer semantics (which I used for the session idempotency test in `7c92670`). `packages/state/STATE-PROTOCOL.md` is the State Protocol v1.0 reference for the materializer rewrite.

## Agent coordination patterns — how to manage the swarm

### Lane ownership

When multiple agents work in parallel, each owns specific files. Before dispatching a subagent, **enumerate the files it will touch** and ensure they do not overlap with any other in-flight lane. Write the file ownership into the subagent's prompt as a hard constraint ("Do not touch X, Y, Z — another lane is working there").

### Lanes I managed this session

1. **Me** (main session) — orchestration cleanups, test promotions, commits, review
2. **Tools schema-only subagent** (`aeaf74e684efd620e`) — `tool_descriptor` envelope emission
3. **Session materialized subagent** (`a965e65d09a236275`) — session materialized-vs-raw test
4. **Harness seven-combinator subagent** (`a670cf479914ff831`) — combinator coverage test
5. **Slice 17 subagent** (`a580732ce1fcc2f84`) — capability profiles + attach_tool
6. **Gnijor's orchestration-regression-fix agent** — `RuntimeHost::pending_runtime_specs` (landed as `b6156d9`)
7. **Gnijor's OTel instrumentation agent** — still in flight at handoff time

All six subagents ran with **no collisions** because I enumerated file ownership in every dispatch prompt.

### Dispatch prompt structure that worked

```
- Repo: absolute path
- Grounding docs to read first (file:line specific)
- What to add (production code, with specific file:line targets)
- What to add (test promotions, with specific file names and function names)
- Hard constraints (list of files NOT to touch, with reasoning)
- Verify commands (cargo check, cargo test --no-run, cargo test -- --nocapture)
- Report structure (under N words, specific fields)
- Outcome options: (A) full success, (B) partial with specific blocker, (C) blocked with minimum needed
```

The outcome-options clause is important — it gives the subagent permission to return honestly when the task is harder than expected, instead of silently weakening the oracle to make a test pass.

### Collision avoidance rule

Before dispatching or editing: run `git status --short`, enumerate modified files, group by lane, confirm the files you're about to touch are in no other active lane. If there's overlap, either coordinate or wait.

### Commit ordering rule

When multiple lanes' changes are in the working tree, commit the most self-contained lane first. Attribute honestly in the commit message if you bundle co-located changes from another lane (as I did for the OTel annotations that had already landed in `src/orchestration.rs` when I committed `e1bd331`).

## Gotchas I learned the hard way

Ordered by how badly they bit me.

### 1. Cargo file-lock contention causes SIGTERM'd test runs

When the slice 17 subagent and my primitives-suite verification were both building, one would hold the Cargo file lock and the other would block for 2+ minutes. If the blocked job is running under a task timeout (default ~5 minutes), it gets SIGTERM'd mid-run with exit code 143. The task completion notification says "exit code 0" because the outer `| tail -15` shell wrapper exits cleanly after the SIGTERM message is consumed — **do not trust the outer exit code alone, always read the tail of the output file**. Background job `b9sso0317` did this to me once.

**Mitigation**: serialize heavy cargo runs. If two agents are both building, wait for one to finish before running the other.

### 2. Background agent wall-clock kills at exit 144

Long-running sequential test loops (e.g., `for i in 1 2 3 4 5; do cargo test ...; done`) hit a ~5-minute wall-clock in the task runner and get killed with exit 144. This happened to me twice early in the session when I was trying to verify stability of `session_durable_stream_survives_runtime_death` under a parallel sweep.

**Mitigation**: run sweeps in smaller batches (3 iterations max), or use `run_in_background: true` on the Bash tool so the task runner doesn't wall-clock you.

### 3. Test oracle races caught by pre-flush assertions

`session_durable_stream_survives_runtime_death` originally failed intermittently because it stopped the runtime before waiting for `prompt_turn` to land on the stream. The oracle was "read after death, assert prompt_turn is there" — but the prompt_turn could be in flight when the runtime was killed. Fix: wait for `prompt_turn` to land via `wait_for_event_count` BEFORE calling `stop_runtime`. Commit `d1e13df`.

**Lesson**: whenever a test does "append, then kill, then read", add an explicit `wait_for_event_count` between the append and the kill. The `wait_for_event_count` / `wait_for_event` helpers in `tests/support/managed_agent_suite.rs` are there for exactly this.

### 4. TOCTOU port races in the harness

`ControlPlaneHarness::spawn` used to call `reserve_port()` which would bind a `TcpListener`, get its port, drop the listener, then pass the port integer to a subprocess. Between drop and subprocess-bind, another parallel test binary could steal the port. Fixed by making the control plane accept `--port 0` and write the actual bound address to a `--listen-addr-file` path. See `crates/fireline-control-plane/src/main.rs` and `tests/support/managed_agent_suite.rs::spawn_control_plane`.

**Lesson**: never bind-and-drop a port for reservation. Always keep the listener and hand the addr through.

### 5. The idempotency test pending message was WRONG

For several check-ins I thought `session_idempotent_append_under_retry` was blocked on a product decision about what idempotency semantics Fireline should commit to. It turned out the durable-streams upstream `PROTOCOL.md` §5.2.1 already pins the contract: Kafka-style `(Producer-Id, Producer-Epoch, Producer-Seq)` transport-layer dedup. The "pending decision" was a misread of the pending_contract message I had written myself earlier. **Read the upstream protocol docs before declaring something blocked on a decision.**

### 6. Durable-streams chunks can bundle everything

For `session_replay_from_mid_offset_is_suffix_of_full_replay` I initially tried to capture a mid-stream cursor by reading the first chunk and using its `next_offset`. Server returned all 13 events in one chunk so there was no "mid" position. Fix: capture the cursor between two prompts — phase 1 prompts, drain to live edge, capture cursor, phase 2 prompts, read from cursor. Also had to use `LiveMode::Sse` on the suffix reader so it tolerates a cursor that happens to match the live edge at capture time. See `7c92670` minus one commit (actually `ab1863a`).

### 7. Tools `#[serde(rename_all_fields)]` is load-bearing

See slice 17 section above. Without that attribute, `TransportRef::PeerRuntime { runtime_key: String }` serializes `runtime_key` as snake_case on the wire, which breaks JSON topology config deserialization. The slice 17 subagent caught this with a regression-guard unit test before it shipped broken. Keep that test.

## Other observations worth keeping

### State projections need `reset()`

When I added control-message handling to the materializer, I added a `reset()` method to the `StateProjection` trait with a default no-op. `SessionIndex` and `ActiveTurnIndex` override it to clear their maps. If you add a new projection, **think about what `reset()` means for it** — the default no-op is probably wrong.

### The `fireline-testy-fs` agent is underused

I built it for the resources fs_backend end-to-end tests, but it's a general scripted-effect test agent. If you need to prove other ACP request emissions (`terminal/create`, `session/request_permission`, etc.), consider extending the `FsTestyCommand` enum rather than building a new agent from scratch.

### `reconstruct_runtime_spec_from_log` has a 5-second deadline

`src/orchestration.rs::reconstruct_runtime_spec_from_log` polls for up to 5 seconds. When the stream is under contention or the materializer is slow, this can be tight. If a test starts flaking with "runtime_spec not found", suspect this deadline before suspecting the materializer logic.

### `ControlPlaneHarness::shared_state_url()` is the ONLY clean way to get that URL

Don't re-derive it. Use the accessor (added in `e1bd331`, defined in `tests/support/managed_agent_suite.rs`). If a test needs the shared state URL, it's available via `control_plane.shared_state_url()`.

### Memory file pointers

The user's auto-memory at `/Users/gnijor/.claude/projects/-Users-gnijor-gurdasnijor-durable-acp-rs/memory/` has some high-signal pointers worth reading on session start:
- `feedback_sdk_first.md` — "Always check rust-sdk APIs before hand-rolling ACP code"
- `feedback_primitives_first.md` — "Read the primitive's contract first (docs/ts/primitives.md + architecture principle + domain doc)"
- `project_transport_bridge.md` — ConductorState dissolution shelved, shared state quick-fix shipped
- `project_architecture_handoff.md` — full architecture handoff context

## Ranked "next move" list

When you resume, pick from this list in order. Each item notes whether it's demo-critical.

### A. Verify the full suite is green after all in-flight lanes land [DEMO-CRITICAL]

The demo is tomorrow. Once the OTel lane commits its remaining 4 files, run:

```bash
cd /Users/gnijor/gurdasnijor/fireline && \
cargo test --test managed_agent_session \
           --test managed_agent_sandbox \
           --test managed_agent_harness \
           --test managed_agent_orchestration \
           --test managed_agent_resources \
           --test managed_agent_tools \
           --test managed_agent_primitives_suite
```

Expected: **27 live tests pass, 4 tests ignored with honest pending messages**. If anything is red, that's the first thing to fix.

Run this as `run_in_background: true` to avoid the 5-minute wall clock kill. Use the output file to check the result.

### B. Honest treatment of the 2 Docker-scoped pending tests

`resources_physical_mount_is_shell_visible_inside_runtime` and `managed_agent_resources_physical_mount_shell_visibility_contract` are both "shell-visible mount" tests that can only be proven in Docker. The honest treatment is to update their `pending_contract` messages to say "covered by `tests/control_plane_docker.rs` slice 13c — this stub is a primitive-coverage cross-reference marker" and maybe rename them to make the cross-reference nature obvious. Do NOT try to promote with a weakened oracle.

This is a ~10-line doc change but it moves the honest count from 27/4 to "27 live + 2 covered-elsewhere + 1 by-design + 1 real gap".

### C. Migrate `peer_mcp` and `smithery` to `attach_tool`

Slice 17 deliberately left the existing bespoke factories in `src/topology.rs`. The logical follow-up is to migrate them to `attach_tool` entries: a `peer_mcp` topology entry becomes `attach_tool { capabilities: [CapabilityRef { transport_ref: PeerRuntime { ... }, ... }] }`. Same for smithery. This is the architectural simplification I called out in the pre-slice-17 recommendation — it removes the "factory per tool source" pattern and makes topology truly data-first.

**Risk**: the migration might break existing tests that use `peer_mcp` directly in their topology specs (e.g., `tools_schema_only_contract` in `tests/managed_agent_tools.rs`). Keep those tests green by either leaving the `peer_mcp` factory as a thin wrapper that constructs the `CapabilityRef` internally, or updating every test fixture. Do NOT break green tests.

### D. Cross-runtime client shim for `harness_durable_suspend_resume_round_trip`

This is the one genuine remaining substrate gap. The test description is in `tests/managed_agent_harness.rs:256-283`. What's needed: a way to start a prompt on runtime A, capture the client's session state before A dies, kill A, re-provision A' from the durable spec, reconnect the client to A', call `session/load` on A', and observe the approval gate's rebuilt state picking up where A left off. This is non-trivial because the client (in tests) normally doesn't survive runtime death — the WebSocket connection closes and the client task completes with an error.

One possible approach: build a "detached client" test helper that holds the original session_id but reconnects its WebSocket to a new ACP URL on demand. Another approach: split the test into two phases — phase 1 proves the paused state is durable in the log (separate from any live client), phase 2 proves a fresh client can `session/load` on a fresh runtime and observe the approval_resolved event on the log. The second shape is cleaner and probably what the test should become.

**Complexity estimate**: 1-2 hours of focused work, not a one-shot.

### E. Slice 17 follow-ups (`TransportRef::McpUrl` live resolver, credentials)

Slice 17 is descriptor-emission only. A follow-up slice needs to actually:
- Resolve `CredentialRef` (`Env { var }` → read env var, `Secret { key }` → read from secret store, `OauthToken` → exchange)
- Connect to `TransportRef::McpUrl { url }` via rmcp's `StreamableHttpClientTransport` and forward tool calls
- Register `TransportRef::InProcess { component_name }` as a pointer into the running conductor's component registry
- Maybe: `TransportRef::PeerRuntime` live dispatch via the peer runtime's ACP surface

Each of these is a small self-contained slice. Dispatching them to subagents would work well.

### F. Not-yet-surfaced architectural concerns from the reviewer

Earlier rounds of review flagged these medium-priority items that are still open:

- **`Endpoint.headers` declared but always `None`** — descriptors should carry auth-bearing endpoints per `crates/fireline-conductor/src/runtime/provider.rs:39`. Small fix, not vision-critical.
- **`LocalFileBackend` path confinement** — falls back to host paths. Reviewer called it "workable as dev backend, not as long-term semantics for portable, bounded Resources". Could rename to `HostFileBackend` or `UnsandboxedLocalFileBackend` to make the boundary explicit.
- **`helperApiBaseUrl` advertised but `src/routes/files.rs` is TODO** — minor.

None of these are demo-critical. Pick them up as cleanup passes when the scoreboard is solid.

## First-5-minutes checklist on session start

1. `cd /Users/gnijor/gurdasnijor/fireline && git log --oneline -20` — see what's happened.
2. `git status --short` — see what's uncommitted. Compare against the "Working tree at handoff" section above to see which lanes have committed since.
3. `cargo test --test managed_agent_session --test managed_agent_sandbox --test managed_agent_harness --test managed_agent_orchestration --test managed_agent_resources --test managed_agent_tools --test managed_agent_primitives_suite` as a background job. Read the output file when it completes.
4. Check on any active subagents via `ls -la /Users/gnijor/.claude/projects/-Users-gnijor-gurdasnijor-durable-acp-rs/b8ac6588-24dd-404a-9334-76a5f57bb5dc/subagents/`. If any are in flight, don't touch files in their lanes.
5. Read this doc's "Ranked next move list" section and pick the highest-leverage move that's not blocked.

## Demo-readiness assessment

**As of this handoff: demo-ready.** All the primitive contracts Anthropic's post specifies have live Rust-substrate tests, with the honest exceptions being one real substrate gap (cross-runtime suspend/resume shim) and a handful of Docker-scoped invariants covered in `control_plane_docker.rs`. The suite has a 27/4 ratio that's only 4 shy of perfect, and every green test is backed by a real oracle not a weakened assertion.

**What to NOT show in the demo**:
- Don't show shell execution inside a local runtime — it doesn't work; mounts only apply under Docker.
- Don't show `TransportRef::McpUrl` with a real external MCP server — the resolver isn't wired yet; that's slice 17 follow-up work.
- Don't show cross-runtime durable suspend/resume — the test shim isn't built.

**What to DO show**:
- The managed-agents-mapping doc's acceptance bars, with all the `[x]` boxes now ticked.
- A live `cargo test --test managed_agent_*` run showing 27/4.
- The `approval_gate` topology component actually blocking a prompt until an external stream write resolves it — that's a crowd-pleaser.
- The `tool_descriptor` envelope emission — `cargo test --test managed_agent_tools -- --nocapture` shows the collision warning in the output, which is a nice visible demonstration of the first-attach-wins rule.
- The materializer control-message handling — the unit tests in `src/runtime_materializer.rs::tests` are short and demonstrate the shape clearly.

## Closing notes

This was a good session. The suite went 12 → 27 live with no weakened oracles, three architectural commits that each simplify rather than patch, and a clean hand-off to the next session. Gnijor is running late to prep a demo — prioritize A (verify) and B (honest Docker treatment) first when you resume. Everything else is forward work that can wait until after the demo.

If you find yourself at a loss, re-read `managed-agents-mapping.md`'s "Fireline as combinators over the primitives" section and the `typescript-typed-functional-core-api.md`'s "Core Position" section. Those are the tuning fork. Every architectural decision this session was a derivative of those two documents.

Good luck.
