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
| Telegram signature (SIGNATURE MOMENT) | ○ `mono-axr.11` TelegramSubscriber as DS active profile | DS Phase 2 (`mono-axr.9`) + Phase 3 (`mono-axr.3` webhook reference) land first; ~6–8h total to demo-ready | Steps 3/4/5 center on composing `telegram()` middleware into agent.ts; bridge-as-example (`mono-thnc.6.3.3–.9`) superseded and closed 2026-04-12 |
| Telegram adapter library reference | ✓ LANDED `d283392` | — | `examples/telegram-bridge/` retained as reference for `@chat-adapter/telegram` imports + adapter boot pattern; reused inside TelegramSubscriber impl |
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
| P11 | Telegram bot reachable | `source deploy/telegram/bridge.env && curl -s "https://api.telegram.org/bot$TELEGRAM_BOT_TOKEN/getMe" \| jq -r '.ok,.result.username'` | prints `true` and `Jessica_fireline_bot`. **If not**, Step 3/4/5 downgrade to `webhook()`-profile fallback or PRE-STAGED |
| P12 | Telegram tokens sourced in operator env | `echo $TELEGRAM_BOT_TOKEN \| head -c 10` | `TELEGRAM_BOT_TOKEN` has a value (10+ chars visible), loaded from `deploy/telegram/bridge.env` (gitignored). `TELEGRAM_CHAT_ID` optional; not needed for DM-driven DS profile. |

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

**Prereq (done before going on stage):** from the repo root, run `pnpm
install` once, then `cargo build --release --bin fireline --bin
fireline-streams`. The live command below assumes those local artifacts
already exist.

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

**Fallback:** if the local bin shims are stale, the operator has a
prewarmed terminal (Pane B backup) and can rerun the same surfaced CLI
path. Reference pack: [docs/demos/fqa-cli-demo-capture.md](./fqa-cli-demo-capture.md),
[`./docs/demos/scripts/replay-fqa-cli.sh`](./scripts/replay-fqa-cli.sh),
and `docs/demos/assets/recordings/fqa-1-cli.mp4`.

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
what a clean run looks like" without breaking. The surfaced CLI bootstrap
fallback remains the same FQA-1 pack from
[docs/demos/fqa-cli-demo-capture.md](./fqa-cli-demo-capture.md).

---

### Step 3 — Compose `telegram()` middleware, DM your agent (SIGNATURE MOMENT)

**Class: LIVE if `mono-axr.11` TelegramSubscriber DS profile green; PRE-STAGED via rehearsal recording otherwise**

This is the moment that reframes the demo — **and** what reframes the
pitch. There is no separate bridge process. Telegram is just another
DurableSubscriber profile, composed into the 15-line spec the same way
`trace()` and `approve()` are.

**Agent file on overhead** (same file as Step 1, one added line):

```ts
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, approve, budget, secretsProxy, telegram } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    budget({ tokens: 2_000_000 }),
    secretsProxy({ ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' } }),
    telegram({ token: 'env:TELEGRAM_BOT_TOKEN', scope: 'tool_calls' }),
  ]),
  agent(['pi-acp']),
)
```

**Action sequence:**

```bash
# Pane A — same fireline invocation, env-loaded token
set -a; source deploy/telegram/bridge.env; set +a
npx fireline docs/demos/assets/agent.ts
```

**What the audience should see:**

- Pane A: host boot, `telegram(): DurableSubscriber profile active
  (polling @Jessica_fireline_bot)` log line
- **Projector pivots to Telegram** — operator DMs the bot:
  `read README.md and summarize it in two sentences`
- Agent replies **in the Telegram chat** with streaming text (message
  edits happening live as the agent thinks)
- Pane C (Betterstack) shows the trace: `fireline.session.created` (keyed
  on the Telegram user) → `fireline.prompt.request` → `fireline.tool.call`
  (read_file) → `fireline.prompt.request` end. Plus a
  `fireline.subscriber.handle` span with
  `fireline.subscriber_name=telegram` on each event → proof the
  DurableSubscriber contract is carrying the chat interaction.

**Then the unkillable-agent beat, on Telegram:**

```bash
# Pane A — while user starts a second, longer prompt on Telegram:
# Ctrl-C the fireline host
# Then immediately relaunch:
npx fireline docs/demos/assets/agent.ts
```

- Telegram chat: reply freezes mid-sentence
- Operator kills + relaunches the host; no container, no sidecar, no
  bridge process
- Within 1–3 seconds the same message edit resumes; the agent finishes
  its thought mid-sentence
- Audience sees: the host process died, the conversation did not — and
  the chat surface reconnects through the same DS profile

**Narration beat:** "You're looking at Telegram. The agent isn't running
a bridge, isn't running a webhook server, isn't running anything other
than what's in that 15-line file. Telegram is a DurableSubscriber profile
— same trait that powers webhooks, approvals, and peer routing. Chat
surfaces are middleware. The substrate is the durable stream underneath,
which is why we killed the process and the conversation kept going."

**DEMO-RISK (largely resolved):** `mono-thnc.2.3` session/load after host
restart — Jeff root-caused at `498fff6`; two fixes LANDED on w19 branch
(`8947083` approval `rebuild_from_log` live-edge stop + `3e22489` docker
state-stream forwarding). Awaiting CI clear + merge. Once on main,
pre-flight P10 flips green.

**Fallbacks (three layers):**
1. If the `telegram()` middleware is buggy post-`mono-axr.11` landing →
   downgrade narration to Step 4-only mode (approval card renders on
   Telegram via the same profile, but the initial prompt runs via a local
   ACP client in Pane B). Narration stays Telegram-forward.
2. If `mono-axr.11` hasn't landed in time → swap `telegram()` for
   `webhook({ url, scope: 'tool_calls' })` and demo the same DS profile
   pattern against a locally-hosted webhook receiver (`mono-axr.3`
   Phase 3 webhook subscriber is the reference impl). Narration: "Same
   shape, different transport — the point is the substrate."
3. If the session/load fix isn't in → play the PRE-STAGED Telegram
   recording of the restart beat.

### Step 4 — Approval card on Telegram (Demo 2 via DS profile)

**Class: LIVE via `mono-axr.11` TelegramSubscriber approval profile; public-client approval harness fallback otherwise**

Continuing inside the same Telegram chat from Step 3, operator DMs a
prompt that triggers the approval policy:

> `delete the build output with rm -rf dist`

**What the audience should see:**

- The agent proposes the `delete_file` / `run_command` tool call but does
  NOT execute
- Telegram renders an **inline keyboard message** with the tool call
  details + Approve / Deny buttons (rendered by the TelegramSubscriber
  active profile)
- Pane C: `fireline.approval.requested` span emitted with
  `fireline.request_id`, `fireline.policy_id`, `fireline.reason`; plus
  a `fireline.subscriber.handle` span with
  `fireline.subscriber_name=telegram`,
  `fireline.completion_key_variant=prompt`
- Operator taps **Approve** on the phone/laptop
- Telegram inline keyboard updates to "Approved by @operator"
- Pane C: `fireline.approval.resolved` span with `fireline.allow=true`,
  `fireline.resolved_by=telegram:@operator`
- Telegram chat continues: tool runs, agent confirms deletion

**Narration beat:** "Same middleware array as Step 1. The approval card
comes from the same `telegram()` profile that's handling the chat
stream. It's one row in the stream being routed to one subscriber — same
shape as a webhook, same shape as an auto-approver. The chat surface is
just a profile on a generic trait."

**Implementation reference:** `mono-axr.11` — TelegramSubscriber active
profile, companion to WebhookSubscriber (`mono-axr.3`) and
AutoApproveSubscriber. Inline keyboard callback writes
`approval_resolved` on the state stream; approval gate's existing
rebuild-from-log unblocks the paused tool call.

**Fallback:** if TelegramSubscriber misbehaves or `mono-axr.11` slips,
operator falls back to the surfaced approval harness in
[docs/demos/fqa-approval-demo-capture.md](./fqa-approval-demo-capture.md):
boot with `node packages/fireline/dist/cli.js run docs/demos/scripts/fqa-approval-harness.ts --port 4540 --streams-port 7574 --state-stream fqa-approval-public`,
then drive allow/deny with `node docs/demos/scripts/replay-fqa-approval.mjs driver-only --acp-url <ws-url> --state-url http://127.0.0.1:7574/v1/stream/fqa-approval-public`.
Pre-staged recording: `docs/demos/assets/recordings/fqa-4-approval.mp4`.
Same durable-streams approval write path; narration pivots to "the
approver is the public client API in this path. The chat is one of many
profiles; the substrate is the same."

---

### Step 5 — Peer reviewer (also `telegram()`) in the same chat

**Class: LIVE once `mono-axr.11` TelegramSubscriber is in agent + reviewer specs; PRE-STAGED via `d543eac` peer replay otherwise**

`docs/demos/assets/reviewer.ts` carries the **same** `telegram()`
middleware in its compose chain. No additional bridge, no additional
bot registration — same `TELEGRAM_BOT_TOKEN` (one bot speaks for the
fleet), role disambiguated via the TelegramSubscriber profile's
message prefix / reply-threading.

```bash
# Pane A — side terminal, same env already loaded
npx fireline docs/demos/assets/reviewer.ts
```

Operator DMs the primary agent on Telegram:

> `ask the reviewer to double-check your last file change`

**What the audience should see:**

- Primary agent announces it's asking the reviewer
- **Reviewer posts its own message in the same Telegram chat**, prefixed
  or threaded by the TelegramSubscriber profile's peer discriminator
- Pane C: trace tree shows `peer.call.out` from primary →
  `peer.call.in` on reviewer, joined by `_meta.traceparent`
  (canonical-ids Phase 4 `429475e` — unified tree, not adjacent traces)
- Reviewer replies; primary continues with the reviewer's annotation

**Narration beat:** "Two agents, same middleware array, same chat.
Multi-agent isn't a new UI or a new product surface — it's the same DS
profile running in two places. That's what OpenClaw-style products mean
when they say 'agent fleet': a bunch of composed specs, not a bunch of
integrated systems."

**Fallback:** play the FQA-5 peer reference pack off-stage plus the
pre-staged Telegram chat screenshot. Use
[docs/demos/peer-to-peer-demo-capture.md](./peer-to-peer-demo-capture.md)
with [`./docs/demos/scripts/replay-peer-to-peer.sh`](./scripts/replay-peer-to-peer.sh)
(landed `d543eac`). This stays the deterministic peer fallback if
`mono-axr.11` parallelism isn't up by rehearsal.

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
| 3 | LIVE **signature** (contingent on `mono-axr.11` TelegramSubscriber profile + `mono-thnc.2.3` fix merged) | Telegram bot authed via @BotFather, env loaded, `docs/demos/assets/agent.ts` has `telegram()` middleware composed in | Host-restart as PRE-STAGED recording if 2.3 fix slips; `webhook()` swap as DS-profile narrative fallback if `.11` slips |
| 4 | LIVE via TelegramSubscriber inline keyboard (`mono-axr.11` approval profile) | bot authed, callback write path through DS trait | Approve/Deny render as recording if profile flakes; public-client approval harness as last-resort |
| 5 | LIVE via `telegram()` in `reviewer.ts` too (`mono-axr.11`) | reviewer spec committed alongside agent.ts, both running against same bot | FQA-5 `d543eac` peer replay + Telegram chat screenshot |
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
- [ ] Step 4 (approval gate) either TelegramSubscriber LIVE or the
      public-client fallback from `docs/demos/fqa-approval-demo-capture.md`
      exercised
- [ ] Step 6 (peer fleet) replay script runs end-to-end against current main

### Timing
- [ ] Seconds-per-step captured in a timing table (target total: <20 min
      stage time excluding narration pauses)
- [ ] No step hung >10s beyond expected response

### Fallback captures
- [ ] `docs/demos/assets/recordings/fqa-1-cli.mp4` exists; surfaced CLI
      fallback is documented in `docs/demos/fqa-cli-demo-capture.md` and
      driven by `./docs/demos/scripts/replay-fqa-cli.sh`
- [ ] `docs/demos/assets/recordings/step-3-resume.mp4` exists (even if the
      LIVE path works, the capture is the Rehearsal 2 fail drill's reference)
- [ ] `docs/demos/assets/recordings/fqa-4-approval.mp4` exists; public
      approval fallback is documented in
      `docs/demos/fqa-approval-demo-capture.md` and driven by
      `node docs/demos/scripts/replay-fqa-approval.mjs`
- [ ] `docs/demos/assets/recordings/step-5-docker-deploy.mp4` exists
- [ ] FQA-5 peer fallback reference remains
      `docs/demos/peer-to-peer-demo-capture.md` +
      `./docs/demos/scripts/replay-peer-to-peer.sh` (`d543eac`); add the
      MP4 path here once the peer recording is on disk

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
  corresponding PRE-STAGED capture. Concrete surfaced fallback packs:
  Step 1 uses [docs/demos/fqa-cli-demo-capture.md](./fqa-cli-demo-capture.md),
  `./docs/demos/scripts/replay-fqa-cli.sh`, and
  `docs/demos/assets/recordings/fqa-1-cli.mp4`; Step 4 uses
  [docs/demos/fqa-approval-demo-capture.md](./fqa-approval-demo-capture.md),
  `node docs/demos/scripts/replay-fqa-approval.mjs`, and
  `docs/demos/assets/recordings/fqa-4-approval.mp4`; Step 5 uses
  [docs/demos/peer-to-peer-demo-capture.md](./peer-to-peer-demo-capture.md)
  and `./docs/demos/scripts/replay-peer-to-peer.sh` (`d543eac`
  reference).
- **Network drops:** prewarmed local fallback (Demo 1+2 run entirely local;
  Demos 3-7 degrade to pre-staged).
- **Model API outage:** pivot to a prompt the local cache can answer; if
  nothing works, skip to Step 7 and close strong on the observation surface
  (which runs off historical stream data, not live model calls).

## Open TODOs (linked to bd beads)

- [x] Lock `agent.ts` + `reviewer.ts` under `docs/demos/assets/` — **DONE** `dbfac8a` (`mono-thnc.6.1`)
- [ ] `--resume` CLI flag verification or concrete working equivalent — `mono-thnc.6.2` (blocked on canonical-ids Phase 5, see `mono-thnc.2.3` for in-scope gap)
- [x] Telegram bot pre-stage — `mono-thnc.6.3.1` LANDED (@Jessica_fireline_bot via @BotFather, token in gitignored `deploy/telegram/bridge.env`)
- [x] Chat SDK adapter reference library — `mono-thnc.6.3.2` LANDED `d283392` (examples/telegram-bridge retained as reference; superseded as demo path)
- [ ] TelegramSubscriber DS active profile — `mono-axr.11` (was `mono-thnc.6.3.3-.9`, superseded 2026-04-12). Companion to `mono-axr.3` WebhookSubscriber + AutoApproveSubscriber. Composed into `docs/demos/assets/agent.ts` + `reviewer.ts` via `telegram({ token, scope })` middleware. Demo signature moment; ~3–4h impl with parallel workers post-DS-Phase-3.
- [ ] Pre-flight checklist dry-run (P1–P10) — `mono-thnc.6.4`
- [ ] Rehearsal 1 — full-live attempt + fallback capture — `mono-thnc.7` (blocked on T2/T3/T4/T5/.6)
- [ ] Rehearsal 2 — deliberate-fail drill — `mono-thnc.8` (blocked on Rehearsal 1)
- [x] FQA-1/4/5 screencast captures — `mono-thnc.9` CLOSED: FQA-1 + FQA-4 landed at `da521ba`; FQA-5 reference remains `d543eac`. Public-surface crash/restart parity remains tracked in `mono-thnc.9.1`
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
