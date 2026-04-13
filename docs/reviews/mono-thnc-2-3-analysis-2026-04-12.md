# mono-thnc.2.3 Architectural Analysis

> Date: 2026-04-12
> Analyst: Architect (Opus 3)
> Bead: `mono-thnc.2.3` — restart-safe `session/load` on embedded-spec Docker runtime
> Evidence: `docs/reviews/smoke-tier-a-local-docker-2026-04-12.md` @ `7839375`
> Analysis only — NO code changes in this dispatch.

## Symptom recap

After `docker stop` + `docker start`:
- Durable-streams data persists on mounted volume ✓
- New runtime registers against same state stream ✓
- Restarted host's `initialize` advertises `loadSession: true` ✓
- Client calls `session/load` with pre-restart `session_id`
- **Call times out at 15s. WebSocket closes with code 1006 (abnormal, no close frame).** ✗

Approval-mid-crash scenario blocked as consequence. Directly threatens Demo 1 ("Unkillable Agent") signature moment.

## Root-cause hypothesis (ranked)

### Primary: embedded-spec boot doesn't forward state-stream URL to the agent subprocess

**Evidence:**

- `src/bin/testy_load.rs:51` reads `FIRELINE_ADVERTISED_STATE_STREAM_URL` env var to populate `state_stream_url`.
- `testy_load::rebuild_session_from_stream` returns `Ok(false)` immediately if `state_stream_url` is `None`.
- The `session/load` handler's fallback then calls `responder.respond_with_error(session_not_found_error(...))`.
- Docker generic control-plane entrypoint (`docker/bin/fireline-host-quickstart-entrypoint.sh`) conditionally forwards `FIRELINE_ADVERTISED_STATE_STREAM_URL` via `--advertised-state-stream-url`.
- Embedded-spec bootstrap `docker/bin/fireline-embedded-spec-bootstrap.ts::buildDirectHostArgs` does NOT include `--advertised-state-stream-url`. It forwards only `--host`, `--port`, `--name`, `--durable-streams-url`, `--state-stream`, `--topology-json`.

**Open question about symptom match:** this hypothesis predicts an **RPC error** response, not the observed timeout + 1006 abnormal close. Two possibilities:

1. This is the FIRST of two causes. Even if fixed, a SECOND failure mode remains (see below).
2. In embedded-spec direct-host mode, `FIRELINE_ADVERTISED_STATE_STREAM_URL` is also how the harness-side approval-gate path knows which stream to read — missing env var could break an earlier stage than the agent handler.

### Secondary: `rebuild_from_log` hangs on default live mode

**Evidence:**

- `crates/fireline-harness/src/approval.rs::rebuild_from_log` builds a reader: `stream.read().offset(Offset::Beginning).build()` — no explicit `.live(...)`.
- The companion `wait_for_approval` reader explicitly sets `.live(LiveMode::Sse)`.
- `testy_load::rebuild_session_from_stream` explicitly sets `.live(LiveMode::Off)`.
- If the default is Sse or LongPoll, `rebuild_from_log`'s `while let Some(chunk) = ... { if chunk.up_to_date { break; } }` loop hangs forever tailing live events.
- The `LoadSessionRequest` handler in approval.rs is: `this.rebuild_from_log(&request.session_id).await?; cx.send_request_to(Agent, request).forward_response_to(responder)` — rebuild runs BEFORE agent forward. A hang here produces exactly the observed symptom (no response → 15s timeout → eventual WS drop → 1006).

**Symptom match:** timeout + abnormal-close matches this hypothesis better than Primary alone.

### Tertiary: direct-host subprocess lifecycle / conductor topology

Direct-host mode (without `--control-plane`) does not run the SessionIndex rehydration path. Session routing state is not rebuilt from `hosts:tenant-{id}` stream. This alone doesn't break `session/load` for a single-host topology (there's only one agent subprocess to route to), but combined with the Primary and Secondary issues, it may compound failure modes.

**Lower confidence; flagging for completeness.**

## Likeliest combined cause

**Secondary hypothesis is the proximate cause of the timeout+1006.** Primary is a *latent* second bug that would surface as `session_not_found` once Secondary is fixed.

Evidence ranking supports fixing BOTH in the same patch:
- Secondary alone → after fix, `session/load` reaches agent, agent returns `session_not_found` because env var unset → still demo-broken.
- Primary alone → `rebuild_from_log` still hangs → still demo-broken.
- Both → `rebuild_from_log` completes → agent receives request → agent rehydrates from stream → success.

## Scope ruling

**Standalone fireline-host + docker bootstrap bug. NOT canonical-ids Phase 5-adjacent.**

- Phase 5 deletes `ActiveTurnIndex` + rewires peer code. No involvement in the `session/load` codepath.
- Approval gate `rebuild_from_log` behavior is orthogonal to canonical-ids refactor (stream-reader live-mode is durable-streams API, not ACP identity).
- Embedded-spec bootstrap shell + env forwarding is pure operational plumbing.
- **Safe to fix in parallel with Phase 4 on w17.** No merge-conflict or architectural overlap.

## Fix complexity estimate

**2-4h** (engineering judgment).

Breakdown:
- `approval.rs::rebuild_from_log` add explicit `.live(LiveMode::Off)`: 15min (one-line).
- `docker/bin/fireline-embedded-spec-bootstrap.ts` add `--advertised-state-stream-url` forwarding: 30min (plus entrypoint shell plumbing if the env var needs to surface from docker env → node → rust).
- Rebuild docker image + re-run smoke reproducer: 30min.
- Validate approval-mid-crash resume (Step 5 from smoke doc): 30min.
- Contingency for debugging additional failure modes: 1-2h.

**If neither hypothesis pans out** — escalate to `"bisection needed"` after 2h. Most likely next suspect would be the conductor topology's proxy forwarding path.

## Fix location

Specific file pointers:

1. **`crates/fireline-harness/src/approval.rs`** line ~212 — `rebuild_from_log` stream reader build. Add `.live(LiveMode::Off)`. Confirm by grepping the other live-mode call sites; match testy_load's pattern (which works).

2. **`docker/bin/fireline-embedded-spec-bootstrap.ts::buildDirectHostArgs`** — add `--advertised-state-stream-url` flag if `FIRELINE_ADVERTISED_STATE_STREAM_URL` env var is set (or synthesize from `FIRELINE_DURABLE_STREAMS_URL` + `stateStream`).

3. **`docker/bin/fireline-host-quickstart-entrypoint.sh`** — export `FIRELINE_ADVERTISED_STATE_STREAM_URL` before invoking the embedded-spec bootstrap. Needs to resolve to the EXTERNALLY-reachable URL the agent subprocess will use (which for a single-container direct-host deployment is `http://127.0.0.1:${FIRELINE_STREAMS_INTERNAL_PORT}/v1/stream/${stateStream}`).

## Regression risk

**LOW** for both the fix and for w17's Phase 4 in flight.

- `approval.rs` fix is one line (`.live(LiveMode::Off)` on rebuild_from_log). Phase 4 edits different approval.rs regions (`_meta` trace handling). No merge conflict expected.
- Docker bootstrap fix is isolated to docker/ shell + tsx bootstrap. Phase 4 doesn't touch docker/.
- Existing approval-gate unit tests (`concurrent_waiters_are_isolated_by_session_and_request_id`) use `LiveMode::Sse` on `wait_for_approval` but `rebuild_from_log` is called via `LoadSessionRequest` only in the `rebuild_from_log` test harness path. Adding explicit `.live(LiveMode::Off)` should not break existing tests (they pre-seed events and terminate cleanly).

**Residual risk:** if the durable-streams `.build()` default IS already `LiveMode::Off`, the Secondary hypothesis is wrong and the true hang is elsewhere (possibly conductor proxy or an SSE reader in a middleware component I haven't audited). Mitigation: before landing, add `tracing::info!` at `rebuild_from_log` entry/exit and verify in the fix PR's smoke log that the function actually returns.

## Recommended owner + path forward

**Owner:** emergency dispatch to any available codex (NOT w17 — keep Phase 4 uninterrupted).

**Verification gate for the fix PR:**
1. Re-run the full smoke from `docs/reviews/smoke-tier-a-local-docker-2026-04-12.md` §5 + §9.
2. Assert `session/load` returns `LoadSessionResponse` (not error, not timeout).
3. Assert WS close code is 1000 (normal) when client calls `ws.close()`.
4. Assert approval-mid-crash resume completes end-to-end — paused prompt unblocks after external `approval_resolved` append.
5. Capture the fixed smoke evidence at `docs/reviews/smoke-tier-a-local-docker-2026-04-12.md` §Followup (append; don't replace original).

**Demo fallback:** if the fix doesn't complete in 2h, operator script Step 3 already documents the PRE-STAGED path for "kill + resume" — use it. LIVE execution is strictly more impressive but not demo-critical given the fallback is scripted.

## References

- [smoke-tier-a-local-docker-2026-04-12.md @ 7839375](./smoke-tier-a-local-docker-2026-04-12.md)
- `crates/fireline-harness/src/approval.rs` (session/load handler + rebuild_from_log)
- `src/bin/testy_load.rs` (session/load handler + rebuild_session_from_stream + env var)
- `docker/bin/fireline-embedded-spec-bootstrap.ts` (embedded-spec direct-host args builder)
- `docker/bin/fireline-host-quickstart-entrypoint.sh` (docker entrypoint env forwarding)
- [alignment-check-2026-04-12.md](./alignment-check-2026-04-12.md)
