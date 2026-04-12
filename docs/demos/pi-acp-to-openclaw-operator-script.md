# pi-acp → OpenClaw — Operator Script (Live Driver)

> Status: skeleton — 2026-04-12, Jessica (PM-B)
>
> This is the stage-side companion to `pi-acp-to-openclaw.md`. The other doc
> describes the *story*; this doc is the *driver*: exactly what the operator
> types, exactly what the audience sees, and exactly which parts are live,
> pre-staged, or mocked.
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

Aligned with the `DEMO-PLAN.md` priority order:

1. **Demo 1 — Unkillable Agent** (signature story, highest confidence)
2. **Demo 2 — Approval Gate** (depends on Slack/webhook glue being stage-safe)
3. **Demo 3 — Live Dashboard** (background backdrop)

Demos 4 (Flamecast in 200 lines) and 5 (Provider Swap) are stretch and not part
of this script.

## Pre-flight checklist (T-60 minutes before stage)

Operator runs through this in the dressing room, not on stage:

| # | Check | Command / action | Pass criterion |
|---|---|---|---|
| P1 | Repo clean on main, latest | `git fetch && git status -sb` | `## main...origin/main` and clean tree |
| P2 | Streams binary present | `which fireline-streams && fireline-streams --version` | prints version |
| P3 | Host binary present | `which fireline && fireline --version` | prints version |
| P4 | Anthropic key loaded | `echo $ANTHROPIC_API_KEY \| head -c 10; echo` | first 10 chars visible |
| P5 | GitHub token loaded (for tools using `api.github.com`) | `echo $GITHUB_TOKEN \| head -c 10; echo` | first 10 chars visible |
| P6 | Demo agent file present | `ls demo/agent.ts demo/reviewer.ts` | both files exist |
| P7 | CF Containers deployment healthy (if Step 3 live) | `wrangler tail <deployment-name> --format pretty` in spare terminal | clean event stream, no crash loop |
| P8 | OTel backend reachable (if Step 5 live) | open Betterstack dashboard in browser; confirm source receiving heartbeat | at least one recent event in last 60s |
| P9 | Backup terminal prewarmed with fallback commands | separate tmux/cmux pane with fallback scripts open | operator can switch in <3s |

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
npx fireline demo/agent.ts
```

**Agent file (`demo/agent.ts`) — shown briefly on overhead:**

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
fail over with `fireline demo/agent.ts` in <3s.

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

### Step 3 — The unkillable agent (Demo 1 signature moment)

**Class: LIVE**

**Action sequence in Pane A:**

```bash
# (In a separate pane, prompt the agent again with something that takes
# multiple tool calls — e.g., "list files in demo/, then read the largest one,
# then summarize")
# Partway through the response streaming into Pane B, operator runs:
kill -9 $(pgrep -f 'fireline .*agent.ts' | head -1)
# (Or Ctrl-C the visible host process in Pane A.)

# Then immediately:
npx fireline demo/agent.ts --resume <session-id-shown-in-step-1>
```

**What the audience should see:**

- Mid-sentence, Pane B freezes (old host dead)
- Pane A shows the old process exit
- Operator re-launches; in 1-3 seconds Pane B resumes streaming *from where the
  sentence stopped*, not from scratch
- Pane C shows two `fireline.session.created` spans bound to the same
  `fireline.session_id`, second is the rehydrated host

**Narration beat:** "The process is disposable. The session isn't. Everything
the agent was doing — midsentence — resumes, because state lives in
durable-streams, not in the host's RAM."

**Fallback:** if `--resume` semantics aren't yet wired into the published CLI
(verify during pre-flight P3), operator uses the lower-level
`fireline --load-session <id>` path or drops this to PRE-STAGED (screen capture
from a rehearsal run). **THIS STEP IS THE SIGNATURE MOMENT — DO NOT DEMO LIVE
IF PRE-FLIGHT P3 IS UNSTABLE.**

---

### Step 4 — Approval gate (Demo 2)

**Class: LIVE (if Slack glue pre-flight P8 passes) otherwise PRE-STAGED**

**Action in Pane A:**

Prompt the agent (via the ACP client) with something that triggers the approval
policy — e.g., "delete `demo/scratch.txt`".

**What the audience should see:**

- Pane B streams: agent proposes the `delete_file` tool call
- Pane B pauses: `approval_requested` event emitted, no execution yet
- A Slack notification appears (on projector or phone) with Approve / Deny buttons
- Operator taps Approve
- Pane B resumes: tool executes, agent confirms deletion

**Narration beat:** "The approval is durable. If I crashed the host right now
between the request and the tap, the approval would still be there when the
host came back up. This isn't a modal — it's infrastructure."

**Fallback:** if the Slack webhook path fails (check pre-flight P8), operator
has a local "approver" CLI tool (`fireline approve --session <id> --request-id <id>`)
that resolves via the same durable-streams write. Narration: "Approver happens
to be a CLI in this path — the Slack app is one of many resolvers on the same
substrate."

---

### Step 5 — Same OCI image, portable target story (Demo 5 teaser)

**Class: depends on T1 + T2 state at showtime**

**Framing:** The demo story here is *portability*, not a specific cloud. The
message is: "`fireline build` produces one OCI image. That image runs locally
under docker, and ships via target-native tooling (flyctl, wrangler, kubectl)
to any container platform." The validated truth at demo time is local docker
(T2); other targets are future work.

**If T1 (`fireline deploy` wrapper) and T2 (local Docker smoke + embedded-spec
entrypoint fix) are BOTH green:**

**Class: LIVE**

```bash
npx fireline build demo/agent.ts                             # produce OCI image
docker run -d --name fireline-demo \
  -p 8087:8087 \
  -v $PWD/.fireline-streams:/var/lib/fireline/streams \
  fireline/demo:latest                                       # run locally
# (endpoint URL printed; client reconnects against it)
```

Audience sees:
- Build log → image SHA
- `docker run` succeeds; Pane B reconnects its ACP client to the mapped port
- Pane C registers a `fireline.sandbox.provisioned` span with
  `fireline.provider=docker-local`
- Operator narrates the per-target extensions: "same image, `fireline deploy
  --to fly` / `--to cloudflare-containers` / `--to k8s` — local validation
  today, per-target validation on the hosted-deployment roadmap."

**If T1 green but T2 not green (entrypoint fix or smoke incomplete):**

**Class: PRE-STAGED.** Operator shows the `fireline build` + `docker run`
command list and plays a screen capture of a rehearsal run. Narration is
unchanged — the portability story does not require the live proof, but the
live proof is stronger.

**If T1 not green:**

**Class: MOCKED.** Show intended commands; cut to Step 6. Narration: "This is
shipping — `fireline build` lands this week; the docker story is already
proven; the CLI wrapper is the last mile."

**CF Containers note:** CF Containers is **not** demoed. It is framed as a
future target, deferred pending object-storage-native durable-streams
protocol. Do not show `--to cloudflare-containers` in any LIVE step unless
explicitly greenlit on the day.

---

### Step 6 — Peer fleet (stretch)

**Class: PRE-STAGED by default; upgradable to LIVE if FQA-5 P2P passes with
room to spare**

Run `reviewer.ts` in a second deploy; show Pane C with both agents appearing
in the fleet surface and a cross-agent call lineage rendered via OTel span
parent/child relationships (T4 `fireline.peer.call.out/in` spans).

**Fallback:** revert to screen capture of a prior rehearsal.

---

### Step 7 — Observation surface close (Demo 3 backdrop)

**Class: LIVE (Betterstack dashboard, already pre-staged in pre-flight P8)**

Operator flips projector to Pane C full screen. Shows:

- the trace tree for the entire demo session (steps 1-6)
- approvals timeline
- per-agent latency chart

**Narration close:** "Every line of that came out of one stream. We didn't
write dashboard code. The dashboard is a projection."

---

## Honesty ledger (what's really live vs theater)

| Step | Honesty class | What's pre-staged | What's mocked |
|---|---|---|---|
| 1 | LIVE | pi-acp binary resident | none |
| 2 | LIVE | none | none |
| 3 | LIVE **(contingent)** | resume session id from Step 1 | if P3 fails: screen capture |
| 4 | LIVE or PRE-STAGED | Slack app registered + webhook URL | Approve/Deny button layout if Slack fails |
| 5 | LIVE / PRE-STAGED / MOCKED (T1+T2 gated) | local docker image pre-built; volume dir prepared | CF Containers deploy (deferred; never LIVE at demo) |
| 6 | PRE-STAGED (default) | second agent already deployed | lineage graph frame |
| 7 | LIVE | dashboard layout + saved view | none |

## Failure recovery plan

- **Any LIVE step hangs >10s beyond expected response:** operator says "let me
  show you what this looks like when it runs clean" and pivots to the
  corresponding PRE-STAGED capture.
- **Network drops:** prewarmed local fallback (Demo 1+2 run entirely local;
  Demos 3-7 degrade to pre-staged).
- **Model API outage:** pivot to a prompt the local cache can answer; if
  nothing works, skip to Step 7 and close strong on the observation surface
  (which runs off historical stream data, not live model calls).

## Open TODOs for this script to be stage-ready

- [ ] Lock `demo/agent.ts` and `demo/reviewer.ts` final form; commit to repo
      under `docs/demos/assets/` so CI verifies they still compile
- [ ] Rehearsal run #1 — full script end-to-end; capture timing per step
- [ ] Record PRE-STAGED fallback captures for steps 3, 4, 5, 6 (one rehearsal
      where everything works cleanly)
- [ ] Confirm `--resume` CLI flag exists or adjust Step 3 to the concrete
      command that works today (pending PM-A canonical-ids status)
- [ ] Pre-stage Slack app + webhook URL for Step 4; validate pre-flight P8
- [ ] Pre-stage CF Containers deployment name + wrangler tail command for
      Step 5 (contingent on T2 landing)
- [ ] Lock Betterstack dashboard saved view URL for Step 7 (contingent on T5)
- [ ] Rehearsal run #2 — deliberately trigger each fallback path and confirm
      recovery stays within narrative
- [ ] Two-operator dry-run (one on keys, one on narration) in final
      staging environment

## References

- `docs/demos/pi-acp-to-openclaw.md` — the narrative
- `docs/DEMO-PLAN.md` — five-demo framing and priorities
- `docs/status/orchestration-status.md` §Jessica (PM-B) dispatch queue — T1–T5
  gating state
- `docs/proposals/observability-integration.md` — OTel Phase 2 span catalog
  (drives Pane C content)
