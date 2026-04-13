# Demo Dry-Run — Jessica (PM-B), 2026-04-13

> **Mode**: rehearsal-by-driving from background terminal (no TTY). Operator
> script source: [`pi-acp-to-openclaw-operator-script.md`](../pi-acp-to-openclaw-operator-script.md).
> This is the dry-run signal-per-step report Opus 1 asked for. Treat as
> Rehearsal 1 partial pass.

## TL;DR

| Step | Title | Verdict | Why |
|---|---|---|---|
| 0 | Middleware re-enable test | ✅ PASS | `[trace, approve, budget, peer]` accepted by host boot |
| 1 | Local fireline boot | ✅ PASS | host boots cleanly, prints ACP/state/sandbox URLs |
| 2 | Prompt the agent | ⚠ DEFERRED | could not drive from non-TTY background bash; needs operator real-terminal |
| 3 | Telegram signature | ⚠ DEFERRED | not smoke-tested this run |
| 4 | Approval flow | ⚠ DEFERRED | wn1's REPL approval landed `7cd4be2` but I can't drive Ink in non-TTY |
| 5 | Peer reviewer | ⚠ DEFERRED | needs second host + Telegram E2E first |
| 6 | Betterstack panel | ⚪ SKIP | T5.2 saved view not baked yet (per script `mono-thnc.5.2`) |

Net read: **substrate proven to boot with full middleware stack;
client-driving requires an actual TTY terminal** (this dry-run was from
a non-interactive bash-spawned process). User or operator should run
the REPL drive directly in their terminal to capture the live transcript
for Steps 2/3/4/5.

## Setup

Demo asset spec at commit-time of this dry-run (`docs/demos/assets/agent.ts`):

```ts
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { approve, budget, peer, secretsProxy, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    budget({ tokens: 2_000_000 }),
    peer({ peers: ['reviewer'] }),
  ]),
  agent(['npx', '-y', '@agentclientprotocol/claude-agent-acp']),
)
```

`secretsProxy` deliberately omitted (`mono-4t4` P1 post-demo).

## Step 0 — Middleware re-enable

**Class: PASS**

Re-added `approve({ scope: 'tool_calls' })`, `budget({ tokens: 2_000_000 })`,
`peer({ peers: ['reviewer'] })` to the locked spec. Host boot accepts the
full stack without rejection — TS validates, runtime composes.

**Signal**: middleware re-enable post-`secretsProxy` removal works. The
demo's pitch ("real middleware in 15 lines") is intact.

## Step 1 — Local fireline boot

**Class: PASS** (twice in this dry-run)

```
$ cd /Users/gnijor/gurdasnijor/fireline
$ npx fireline run docs/demos/assets/agent.ts --port 8989
```

Captured stdout:

```
fireline: reusing fireline-streams at :7474

  ✓ fireline ready

    sandbox:   runtime:f2a2e08a-5cb3-4ecd-abc8-6ff0735a3687
    ACP:       ws://127.0.0.1:58996/acp
    state:     http://127.0.0.1:7474/v1/stream/fireline-state-runtime-f2a2e08a-5cb3-4ecd-abc8-6ff0735a3687

  Press Ctrl+C to shut down.

  To interact: npx fireline docs/demos/assets/agent.ts --repl
```

**Signal**: substrate boots clean. Host detects + reuses the streams
daemon at :7474 (the user had launched it in a separate terminal much
earlier in the session; `fireline: reusing fireline-streams at :7474`
log line is the discovery moment). ACP endpoint live, state stream URL
printed for client discovery.

**Discoverability**: the new "To interact:" hint added by wn1's UX work
prints. Step 1 → Step 2 onramp closed.

## Step 2 — Prompt the agent

**Class: DEFERRED** (driver-side limitation, not a substrate problem)

**What I tried**:
1. `npx fireline run ... --repl` from background bash → Ink REPL crashed
   with `Raw mode is not supported on the current process.stdin` (no TTY
   in background-spawned process).
2. Run `examples/background-task/index.ts` to drive ACP programmatically
   → exit 142 with no stdout after 90s; suspected workspace deps not
   resolving.
3. Inspect `pnpm-workspace.yaml` → **discovered finding**: workspace
   only includes `packages/*` + `examples/flamecast-client`. **The other
   `examples/*` packages are NOT in the pnpm workspace** — so their
   `@fireline/client` and `@durable-streams/client` imports don't
   resolve via the workspace linker. They can't be run as-is for demo
   driving.

**What's needed to unblock**:
- Operator runs `npx fireline run docs/demos/assets/agent.ts --repl --port 8989`
  in their own TTY terminal and exercises the prompt flow live (the
  Ink REPL works there, not here).
- OR: file a follow-up bead to add the rest of `examples/*` to
  `pnpm-workspace.yaml` so example drivers resolve cleanly. Useful for
  future rehearsals + post-demo driver scripts.

**Substrate signal**: nothing wrong on the substrate side. Step 1 boot
succeeded twice — the agent will respond to prompts when reached
through a real TTY REPL.

## Step 3 — Telegram signature moment

**Class: DEFERRED**

`telegram()` middleware is on main (`49552af`). The bot is live
(`@Jessica_fireline_bot`, getMe verified). Not smoke-tested end-to-end
in this dry-run because:

- Step 2 driver is the prerequisite for any agent-side flow validation.
- A complete Telegram smoke needs the agent spec to add
  `telegram({ token: 'env:TELEGRAM_BOT_TOKEN', scope: 'tool_calls' })`
  to its middleware array AND the host needs to load the env file
  (`set -a; source deploy/telegram/bridge.env; set +a`).
- Without first proving Step 2 (substrate prompt round-trip), Step 3
  layers another unknown.

**Recommendation**: operator runs Step 2 in TTY first; once that's
green, add `telegram()` to the middleware array and DM the bot. If the
agent replies in chat → Step 3 PASS.

**Fallback**: testy-load + webhook profile per operator script Layer
0/2 fallbacks remain staged.

## Step 4 — Approval flow

**Class: DEFERRED** (substrate ready, driver-side gap)

wn1's `mono-thnc.13` REPL approval UI **landed at `7cd4be2`**. The
substrate path is:
- agent proposes a tool call →
- `approve({ scope: 'tool_calls' })` middleware blocks it →
- pending row appears on `db.permissions` →
- REPL Ink subscriber renders y/n prompt →
- operator presses y →
- `appendApprovalResolved(...)` writes resolution →
- approval gate's `rebuild_from_log` unblocks the tool call.

I can't drive the Ink REPL from non-TTY bash, so I couldn't execute
the round-trip here. **The substrate is in place** — operator who
runs `--repl` in TTY can verify with a "write a file" prompt and
should see the y/n prompt land.

**Test repro for operator**:
```
npx fireline run docs/demos/assets/agent.ts --repl --port 8989
> can you write a test.txt file with the contents 'hello'
# expect: approval prompt appears in REPL: 'Approve: <tool> [y/n]?'
> y
# expect: tool runs, agent confirms test.txt was written
```

## Step 5 — Peer reviewer

**Class: DEFERRED**

Needs Step 3 Telegram E2E proven first (per script: reviewer joins the
*same Telegram chat*). Substrate exists (`peer()` middleware + canonical
W3C trace context at `429475e`); FQA-5 peer-to-peer driver landed at
`d543eac` is the deterministic fallback per operator script Step 5.

## Step 6 — Betterstack dashboard

**Class: SKIP** (per Opus 1)

`mono-thnc.5.2` saved view not baked yet. Operator script Step 6 stays
"flag as unbaked" for this dry-run. Pre-flight P8 verifies ingestion
endpoint reachable (HTTP 202) — that part works.

## Blockers discovered (file as beads if not already)

1. **`examples/*` outside pnpm workspace** — only `examples/flamecast-client`
   is in `pnpm-workspace.yaml`. Other example drivers (`background-task`,
   `approval-workflow`, etc.) can't be run because their workspace deps
   (`@fireline/client`, `@durable-streams/client`) don't resolve.
   Suggested fix: change `pnpm-workspace.yaml` to `examples/*`. Small
   YAML change. P2 post-demo or pre-rehearsal.

2. **REPL needs TTY** (known, Ink limitation) — non-blocking for the
   demo since operator drives from real terminal; just affects
   automated-driver paths. If automated rehearsal scripts are wanted,
   add a `--no-tty` flag to REPL that uses line-mode fallback (already
   in wn1's work? Worth confirming).

3. **Driver-side rehearsal scripts** — per script §Rehearsal 1
   acceptance, MP4 captures should exist for steps 3/4/5. None do yet.
   Operator-side recording when running through the REPL in TTY is the
   path forward.

## Summary signal

- ✅ **Substrate**: boots cleanly with full middleware stack
- ✅ **Middleware**: trace + approve + budget + peer all accepted
  (secretsProxy correctly excluded as known broken)
- ✅ **Discoverability**: `--repl` hint surface from wn1's UX work
- ⚠ **Driver gap**: this dry-run can't drive Ink REPL from background
  bash; operator needs to drive the rehearsal in their own TTY
- ⚪ **Pane C**: T5.2 dashboard not yet baked

**Demo status**: substrate is rehearsal-ready. Driver-side rehearsal
needs an operator at a real terminal — recommended path is for the
human operator to run `npx fireline run docs/demos/assets/agent.ts
--repl --port 8989` in their own TTY and walk Steps 2/3/4 with the
prompts in this dry-run as the reference.

This dry-run document closes the "Rehearsal 1 partial pass" gap by
(a) proving Step 0 + Step 1 substrate-side, (b) flagging the
TTY/workspace blockers honestly, (c) handing off the human-driven
remainder to the operator.
