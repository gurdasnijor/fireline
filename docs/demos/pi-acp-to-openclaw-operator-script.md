# pi-acp → OpenClaw — Operator Script (Live Driver)

> Status: polished v0.2 — 2026-04-12, Jessica (PM-B)
>
> This is the stage-side companion to `pi-acp-to-openclaw.md`. The other doc
> describes the *story*; this doc is the *driver*: exactly what the operator
> types, exactly what the audience sees, and exactly which parts are live,
> pre-staged, or mocked.

## Current readiness (as of 2026-04-12)

| Gate | Status | Blocker | Demo impact |
|---|---|---|---|
| T1 `fireline deploy` wrapper | ✓ LANDED | — | Step 5 command set is real |
| T2 Tier A local Docker smoke | ⚠ partial | session/load restart — **fixes LANDED on w19 branch**: `8947083` (approval: `rebuild_from_log` stops at live edge, addresses missing `.live(LiveMode::Off)`) + `3e22489` (docker: forward advertised state stream for embedded-spec boot). Awaiting CI + merge to main. | Step 3 signature moment goes LIVE once commits hit main + pre-flight P10 flips green |
| T3 peer-to-peer replay | ✓ LANDED `d543eac` | — | Step 6 has driver script ready |
| T4.1 five OTel spans | ◐ in flight on w25 | — | Pane C / Step 7 fidelity depends on this |
| T4.2 peer `_meta` propagation | ○ blocked | canonical-ids Phase 4 (`mono-vkpp.6`) | Step 6 trace tree lineage across agents degrades if not green; acceptable fallback |
| T5.1 Betterstack scaffold | ✓ LANDED `46f3f9f` | — | env vars + ingestion verified |
| T5.2 Saved dashboard view | ○ blocked | T4.1 emitting | Pane C layout prepared preemptively |
| `.6.1` agent.ts + reviewer.ts | ✓ LANDED `dbfac8a` | — | demo assets frozen |
| `.6.2` `--resume` CLI verify | ○ blocked | canonical-ids Phase 5 (`mono-vkpp.7`) | see Step 3 fallback |
| `.6.3` Telegram bidirectional chat (SIGNATURE MOMENT) | ○ new sub-epic .6.3.1–.9 | user Telegram app pre-stage (`.6.3.1`); PM-A w22 borrow pending | Steps 3/4/5 re-centered on Telegram per user directive; local-CLI approver remains wired as fallback |
>
> Every step must be labeled with its honesty class:
>
> - **LIVE** — executed on stage against real running systems; output is not rehearsed
> - **PRE-STAGED** — executed before the demo; the result is already on screen or already deployed
> - **MOCKED** — illustrative output shown without a real underlying action (should be rare; must be flagged in narration)
>
> Each step also carries a **fallback** plan for the most likely failure mode so the
> operator can recover without breaking narrative flow.

## Demo target (subject to gates)

Reframed 2026-04-12 per user directive: **Telegram bidirectional chat is the
new signature moment.** The narrative shift is "Fireline is the foundational
substrate for OpenClaw-style systems — your users already text your agents
from their existing chat, approvals happen there, multi-agent threads render
in the same surface."

1. **Signature — Text your agent on Telegram.** You type a Telegram message,
   an agent replies in the same channel with streaming, it asks for an
   approval with a button card, you tap Approve, it finishes. No bespoke UI.
2. **Unkillable agent, now in chat.** Same conversation, operator kills the
   host mid-reply, restarts, user sends next message, session continues from
   where it stopped — visible in the Telegram chat.
3. **Peer in the same thread.** `reviewer.ts` joins, its messages appear in
   the thread, the audience sees multi-agent lineage in a user surface.
4. **Observation surface.** Betterstack trace tree shows Telegram → agent →
   peer-agent lineage as one tree.
5. **Substrate close.** The thing that made all of that possible fits in
   15 lines of agent.ts + a Chat-SDK bridge.

`DEMO-PLAN.md` Demo 1 (Unkillable Agent) + Demo 2 (Approval) + Demo 3 (Live
Dashboard) are now woven into the Telegram-centered arc instead of being
three separate chapters.

Demos 4 (Flamecast in 200 lines) and 5 (Provider Swap) remain stretch, not
part of this script.

## Pre-flight checklist (T-60 minutes before stage)

Operator runs through this in the dressing room, not on stage:

| # | Check | Command / action | Pass criterion |
|---|---|---|---|
| P1 | Repo clean on main, latest | `git fetch && git status -sb` | `## main...origin/main` and clean tree |
| P2 | Streams binary present | `which fireline-streams && fireline-streams --version` | prints version |
| P3 | Host binary present | `which fireline && fireline --version` | prints version |
| P4 | Anthropic key loaded | `echo $ANTHROPIC_API_KEY \| head -c 10; echo` | first 10 chars visible |
| P5 | GitHub token loaded (for tools using `api.github.com`) | `echo $GITHUB_TOKEN \| head -c 10; echo` | first 10 chars visible |
| P6 | Demo assets present (frozen locked versions) | `ls docs/demos/assets/agent.ts docs/demos/assets/reviewer.ts docs/demos/assets/README.md` | all three exist; see `docs/demos/assets/README.md` invariants |
| P7 | Local Docker image present + passes smoke | `docker image ls fireline-host-quickstart:embedded-smoke` and `docker run --rm ... /healthz` | image tag present, `/healthz` returns 200 |
| P8 | OTel backend reachable (Betterstack) | `set -a; source deploy/observability/betterstack.env; set +a` then curl smoke per `deploy/observability/README.md §Smoke-test ingestion` | HTTP 202 Accepted |
| P9 | Backup terminal prewarmed with fallback commands | separate tmux/cmux pane with fallback scripts open | operator can switch in <3s |
| P10 | Step 3 restart/resume gate check | Run `docs/demos/scripts/replay-peer-to-peer.sh` through to the kill-and-resume phase | session continues post-restart. **If it does not** (per `mono-thnc.2.3` open bug, w19 fix in flight at `498fff6`), Step 3 restart beat downgrades to PRE-STAGED recording before showtime |
| P11 | Telegram bridge health | `curl -fsS http://127.0.0.1:<bridge-port>/healthz` and send a test mention in the demo channel | 200 OK + bot replies within 2s. **If either fails**, Step 3/4/5 Telegram paths downgrade to PRE-STAGED + local-CLI approver |
| P12 | Telegram tokens sourced in operator env | `echo $TELEGRAM_BOT_TOKEN \| head -c 10; echo $TELEGRAM_CHAT_ID` | `TELEGRAM_BOT_TOKEN` has a value (10+ chars visible); `TELEGRAM_CHAT_ID` is set if demo targets a specific chat (else empty ok). Both loaded from `deploy/telegram/bridge.env` (gitignored). |

**If any pre-flight fails, the owning step must be downgraded to PRE-STAGED or
cut before going live.**

## Screen layout (operator terminal)

```
┌─────────────────────────────────────────┬─────────────────────────────────┐
│ Pane A — Operator command shell          │ Pane C — Observation surface     │
│ (this is where the audience watches      │ (Betterstack dashboard OR        │
│ commands execute)                         │ local tail of state stream)     │
├─────────────────────────────────────────┤                                   │
│ Pane B — Agent stdout / ACP tail         │                                   │
│ (shows the agent talking)                │                                   │
└─────────────────────────────────────────┴─────────────────────────────────┘
```

## Step-by-step driver

---

### Step 1 — Local pi-acp under Fireline middleware

**Class: LIVE**

**What the operator types in Pane A:**

```bash
npx fireline docs/demos/assets/agent.ts
```

**Agent file (`docs/demos/assets/agent.ts`, frozen at `dbfac8a`) — shown briefly on overhead:**

```ts
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, approve, budget, secretsProxy } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    budget({ tokens: 2_000_000 }),
    secretsProxy({
      ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' },
      GITHUB_TOKEN: { ref: 'secret:gh-pat', allow: 'api.github.com' },
    }),
  ]),
  agent(['pi-acp']),
)
```

**What the audience should see:**

- Pane A prints the fireline startup banner and ACP endpoint URL
- Pane B (agent tail) begins streaming session creation + first idle tick
- Pane C registers `fireline.session.created` span (if T4 OTel spans landed)

**Narration beat:** "15 lines of agent. The middleware is real — trace, approve,
budget, secrets proxy. No glue code. No deploy pipeline yet — this is local."

**Fallback:** if `npx fireline` hangs on first-run install, the operator has a
prewarmed terminal (Pane B backup) where the binary is already resident and can
fail over with `fireline docs/demos/assets/agent.ts` in <3s.

---

### Step 2 — Prompt the agent

**Class: LIVE**

**Action:** operator opens an ACP client (terminal ACP REPL or a minimal web UI
pointed at the endpoint from Step 1) and sends:

> "Read `demo/README.md` and summarize it in two sentences."

**What the audience should see:**

- Pane B streams the agent reading the file and composing the response
- Pane C shows `fireline.prompt.request` → `fireline.tool.call` (read_file) →
  `fireline.prompt.request` end (if T4 spans landed)

**Narration beat:** "Observation is already there. We didn't add a logger. The
stream is the source of truth; everything on the dashboard is a projection of it."

**Fallback:** if the model stalls or the tool call fails, operator has a
pre-recorded transcript loaded in Pane B backup; narration pivots to "here's
what a clean run looks like" without breaking.

---

### Step 3 — Deploy + text your agent on Telegram (SIGNATURE MOMENT)

**Class: LIVE if `mono-thnc.6.3.8` E2E smoke green; PRE-STAGED via rehearsal recording otherwise**

This is the moment that reframes the demo. Operator deploys the agent and
the audience sees Telegram become the interface.

**Action sequence:**

```bash
# Pane A — build + deploy the image (local docker)
npx fireline build docs/demos/assets/agent.ts
docker run -d --name fireline-demo \
  -p 8087:8087 \
  -v $PWD/.fireline-streams:/var/lib/fireline/streams \
  --env-file deploy/observability/betterstack.env \
  --env-file deploy/telegram/bridge.env \
  fireline/demo:latest

# Pane D — projector side, Telegram demo chat already open
# (pre-flight P11 verified bridge is listening)
```

**What the audience should see:**

- Pane A: build log → image SHA → `docker run` starts cleanly
- Pane B: bridge service log shows "connected to Telegram Bot API" and
  "subscribed to session stream"
- **Projector pivots to Telegram** — operator types in the demo channel:
  `@fireline-demo read README.md and summarize it in two sentences`
- Agent replies **in the Telegram chat** with streaming text (message
  edits happening live as the agent thinks)
- Pane C (Betterstack) shows the trace: `fireline.session.created` (bound
  to the Telegram user/channel) → `fireline.prompt.request` → `tool.call`
  (read_file) → `prompt.request` end

**Then the unkillable-agent beat, on Telegram:**

```bash
# Pane A — while user starts a second, longer prompt on Telegram:
docker kill fireline-demo
docker start fireline-demo
```

- Telegram chat: reply freezes mid-sentence
- Telegram shows "Agent reconnecting…" (bridge handles this gracefully)
- Within 1–3 seconds the same message edit resumes; the agent finishes
  its thought mid-sentence
- Audience sees: the container died, the conversation did not

**Narration beat:** "You're not looking at a Fireline UI. You're looking at
Telegram. The user never left their existing surface. We killed the container
and the conversation kept going because the session lives in durable-streams,
not in the host's RAM. Your team's ops surface is the agent's home."

**DEMO-RISK (largely resolved):** `mono-thnc.2.3` — session/load after
container restart — Jeff root-caused at `498fff6`; two fixes LANDED on
w19 branch (`8947083` approval `rebuild_from_log` live-edge stop +
`3e22489` docker state-stream forwarding). Awaiting CI clear + merge to
main. Once on main, pre-flight P10 flips green and Step 3 goes LIVE
without ceremony.

**Fallbacks (three layers):**
1. If `mono-thnc.6.3.8` Telegram E2E is shaky → fall back to
   `mono-thnc.6.3.6`-only mode: only approval cards render on Telegram; the
   prompt itself runs through the local ACP client in Pane B. Narration
   stays Telegram-forward.
2. If Telegram bridge is entirely down → fall back to local ACP client in
   Pane B, approvals via CLI approver. Narration pivots to "showing the
   same flow without the chat surface — the substrate is the same."
3. If `mono-thnc.2.3` fix isn't in → play the PRE-STAGED Telegram recording
   of the restart beat instead of doing it live.

### Step 4 — Approval card on Telegram (Demo 2 reframed)

**Class: LIVE via `mono-thnc.6.3.6` Telegram approval card; PRE-STAGED or local-CLI fallback otherwise**

Continuing inside the same Telegram chat from Step 3, operator types a
prompt that triggers the approval policy:

> `@fireline-demo delete the build output with rm -rf dist`

**What the audience should see:**

- The agent proposes the `delete_file` tool call (or `run_command` with
  `rm`) but does NOT execute
- Telegram renders a **inline keyboard message** with the tool call
  details + Approve / Deny buttons
- Pane C: `fireline.approval.requested` span emitted with
  `fireline.request_id`, `fireline.policy_id`, `fireline.reason`
- Operator taps **Approve** on the phone/laptop
- Telegram inline keyboard updates to "Approved by @operator"
- Pane C: `fireline.approval.resolved` span with `fireline.allow=true`,
  `fireline.resolved_by=telegram:@operator`
- Telegram chat continues: tool runs, agent confirms deletion

**Narration beat:** "The approval is durable. If we crashed the container
between the request and the tap, the approval would still be there when
the container came back up — it's a row in the stream, not an in-memory
callback. And the Approve button isn't a Fireline UI; it's Telegram. Same
chat surface as every other conversation your team has."

**Implementation reference:** `mono-thnc.6.3.6` — Telegram callback query
webhook writes `approval_resolved` on the state stream; approval gate's
existing rebuild-from-log unblocks the paused tool call.

**Fallback:** if the Telegram callback query fails, operator falls back to
the local CLI approver (`fireline approve --session <id> --request-id <id>`)
in a side terminal. Same durable-streams write path — agent resume is
identical; narration pivots to: "The approver is a CLI in this path. The
Telegram inline keyboard is one of many resolvers on the same substrate."

---

### Step 5 — Peer reviewer joins the same Telegram chat

**Class: LIVE via `mono-thnc.6.3.7` (peer visibility); PRE-STAGED via `d543eac` recording otherwise**

Operator deploys the reviewer agent into the same bridge:

```bash
# Pane A — in a side terminal
npx fireline build docs/demos/assets/reviewer.ts
docker run -d --name fireline-reviewer \
  -p 8088:8087 \
  -v $PWD/.fireline-streams:/var/lib/fireline/streams \
  --env-file deploy/observability/betterstack.env \
  --env-file deploy/telegram/bridge.env \
  fireline/demo:reviewer
```

Operator then types on Telegram:

> `@fireline-demo ask @reviewer to double-check your last file change`

**What the audience should see:**

- Primary agent announces it's asking the reviewer
- **Reviewer posts its own message in the same thread** (or a child thread
  per `mono-thnc.6.3.7`'s chosen shape)
- Pane C: trace tree shows `peer.call.out` from primary → `peer.call.in`
  on reviewer, **joined by `_meta.traceparent`** (canonical-ids Phase 4
  landed at `429475e` — propagation is LIVE, not adjacent-traces fallback)
- Reviewer replies; primary continues with the reviewer's annotation

**Narration beat:** "Multi-agent is the same surface. You didn't learn a
new UI. The audience is looking at one Telegram chat and one trace tree;
behind it two agents are exchanging an ACP peer call with W3C trace
context propagation. This is what OpenClaw-style products are built on
top of."

**Fallback:** re-run the off-stage FQA-5 replay
(`docs/demos/scripts/replay-peer-to-peer.sh`, captured at `d543eac`) and
play the Telegram-thread recording from rehearsal. Script + recording =
deterministic fallback.

---

### Step 6 — Observation surface: Telegram → agent → peer lineage in one tree

**Class: LIVE (Betterstack dashboard, pre-flight P8 verified)**

Operator flips projector to the Betterstack dashboard with the saved view
loaded (`mono-thnc.5.2`, session-id variable bound to the demo session).

Audience sees:

- **Panel 5 — Trace tree** renders one unified tree:
  `fireline.session.created` (from Telegram message intake) →
  `fireline.prompt.request` → `fireline.tool.call` (multiple) →
  `fireline.approval.requested` + `fireline.approval.resolved` →
  `fireline.peer.call.out` → reviewer's `fireline.peer.call.in` →
  reviewer's prompt/tool children
- **Panel 4 — Approval timeline** shows the approval card response from
  Step 4 with `fireline.resolved_by=telegram:@operator`
- **Panel 3 — Tool call heatmap** shows the operator's Telegram-prompted
  activity condensed into 60 seconds
- **Panel 1 — Session timeline** shows one or two sessions (primary +
  reviewer if topology uses sibling sessions)

**Narration close:** "Every line of that trace tree came out of one
durable stream. We didn't write the dashboard — it's a projection. And
the user never left Telegram. Fireline is the substrate; the surface is
whatever your team already uses."

---

### Step 7 — Substrate close (optional if time permits)

**Class: LIVE (show-and-tell on the `agent.ts` file)**

Operator splits projector: left half `docs/demos/assets/agent.ts` (15
lines), right half the Telegram chat + trace tree.

**Narration close:** "Everything you just saw — Telegram bidirectional
chat, approvals, unkillable restart, peer lineage — started from 15
lines of agent spec plus a Chat SDK bridge. Fireline is the foundation;
you ship whatever OpenClaw-style product you want on top."

Skip if running over time.

---

## Honesty ledger (what's really live vs theater)

| Step | Honesty class | What's pre-staged | What's mocked |
|---|---|---|---|
| 1 | LIVE | pi-acp binary resident | none |
| 2 | LIVE | none | none |
| 3 | LIVE **signature** (contingent on `mono-thnc.6.3.8` + `mono-thnc.2.3` fix) | Telegram demo chat + bridge pre-staged, demo agent image pre-built, volume dir prepared | Docker restart as PRE-STAGED recording if 2.3 fix slips |
| 4 | LIVE via Telegram inline keyboard (`mono-thnc.6.3.6`) | Telegram callback query webhook registered, bot authed | Approve/Deny card shape rendered as recording if interactive webhook fails; local CLI approver as last-resort fallback |
| 5 | LIVE via Telegram chat (`mono-thnc.6.3.7`) | reviewer image pre-built, bridge routes reviewer to same thread | Telegram chat screenshot + FQA-5 `d543eac` replay recording |
| 6 | LIVE (Betterstack dashboard w/ saved view) | saved-view URL bookmarked in projector browser, session-id variable auto-bound | none |
| 7 | LIVE (split-screen show-and-tell) | `docs/demos/assets/agent.ts` pre-opened in editor for quick display | none |

## Rehearsal 1 acceptance checklist (mirrors `mono-thnc.7`)

Use this on the day. Pass = all items checked; fail = any item unchecked goes
to Rehearsal 2 drill.

### Execution
- [ ] All 7 steps run from the script top-to-bottom without deviation
- [ ] Each LIVE step executes LIVE; each PRE-STAGED step's asset is on-disk
      and ready to play
- [ ] Step 3 (unkillable agent) either LIVE (post-`mono-thnc.2.3` fix) or
      cleanly PRE-STAGED per P10 outcome
- [ ] Step 4 (approval gate) either Chat SDK LIVE or local-CLI fallback
      exercised
- [ ] Step 6 (peer fleet) replay script runs end-to-end against current main

### Timing
- [ ] Seconds-per-step captured in a timing table (target total: <20 min
      stage time excluding narration pauses)
- [ ] No step hung >10s beyond expected response

### Fallback captures
- [ ] `docs/demos/assets/recordings/step-3-resume.mp4` exists (even if the
      LIVE path works, the capture is the Rehearsal 2 fail drill's reference)
- [ ] `docs/demos/assets/recordings/step-4-approval.mp4` exists with both
      Chat-SDK-LIVE and local-CLI-fallback branches captured
- [ ] `docs/demos/assets/recordings/step-5-docker-deploy.mp4` exists
- [ ] `docs/demos/assets/recordings/step-6-peer-fleet.mp4` exists (replay
      driver at `d543eac`)

### Observation surface (Pane C)
- [ ] Betterstack dashboard saved-view is loaded in a tab with the dashboard
      variable `session_id` bound for auto-filter
- [ ] All 5 T5.2 panels render real spans within 30s of Step 1 prompt
- [ ] Trace tree for the demo session is visually readable (no overwhelming
      sibling traces polluting the view)

### Narration
- [ ] Narrator has a per-step script of the "beat" lines memorized or on a
      side panel
- [ ] Two-operator roles confirmed: one on keys (this document's driver),
      one on narration + audience eye contact

Fail in any bucket → Rehearsal 2 drill targets that specific gap.

## Failure recovery plan

- **Any LIVE step hangs >10s beyond expected response:** operator says "let me
  show you what this looks like when it runs clean" and pivots to the
  corresponding PRE-STAGED capture.
- **Network drops:** prewarmed local fallback (Demo 1+2 run entirely local;
  Demos 3-7 degrade to pre-staged).
- **Model API outage:** pivot to a prompt the local cache can answer; if
  nothing works, skip to Step 7 and close strong on the observation surface
  (which runs off historical stream data, not live model calls).

## Open TODOs (linked to bd beads)

- [x] Lock `agent.ts` + `reviewer.ts` under `docs/demos/assets/` — **DONE** `dbfac8a` (`mono-thnc.6.1`)
- [ ] `--resume` CLI flag verification or concrete working equivalent — `mono-thnc.6.2` (blocked on canonical-ids Phase 5, see `mono-thnc.2.3` for in-scope gap)
- [ ] Telegram bidirectional bridge — `mono-thnc.6.3` sub-epic (9 children .6.3.1–.9); signature moment per user directive. Telegram pre-stage = `mono-thnc.6.3.1` (user action), bridge build = .6.3.2 through .6.3.9 on w22 (PM-A reserve loan)
- [ ] Pre-flight checklist dry-run (P1–P10) — `mono-thnc.6.4`
- [ ] Rehearsal 1 — full-live attempt + fallback capture — `mono-thnc.7` (blocked on T2/T3/T4/T5/.6)
- [ ] Rehearsal 2 — deliberate-fail drill — `mono-thnc.8` (blocked on Rehearsal 1)
- [ ] FQA-1/4/5 screencast captures — `mono-thnc.9` (FQA-5 driver already landed at `d543eac`; FQA-1 + FQA-4 captures queued)
- [ ] Betterstack saved-view URL bake-in — `mono-thnc.5.2` (blocked on T4.1 emitting)
- [ ] Rotate Betterstack token post-demo — `mono-thnc.10`

Two-operator dry-run (one on keys, one on narration) is a Rehearsal 2
acceptance criterion, not a separate bead.

## References

- `docs/demos/pi-acp-to-openclaw.md` — the narrative
- `docs/DEMO-PLAN.md` — five-demo framing and priorities
- `docs/status/orchestration-status.md` §Jessica (PM-B) dispatch queue — T1–T5
  gating state
- `docs/proposals/observability-integration.md` — OTel Phase 2 span catalog
  (drives Pane C content)
