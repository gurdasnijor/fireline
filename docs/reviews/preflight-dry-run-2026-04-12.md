# Pre-flight Dry Run (2026-04-12)

Date: 2026-04-12

Scope: dry-run the pre-flight checklist in
[`docs/demos/pi-acp-to-openclaw-operator-script.md`](../demos/pi-acp-to-openclaw-operator-script.md)
items P1 through P12 against current `origin/main`.

Method:

- tracked-file checks were run from an isolated main-based worktree at
  `/tmp/fireline-w24-preflight`
- local operator-machine checks used the real local environment and local
  build artifacts under `/Users/gnijor/gurdasnijor/fireline`
- gitignored env files such as `deploy/telegram/bridge.env` were read from
  the local checkout because they are not copied into a fresh worktree

## Overall Result

Overall status: `PARTIAL`

Passes:

- P1 repo clean/up to date equivalent check
- P4 Anthropic key loaded
- P6 demo assets present
- P7 local Docker image present and `/healthz` passes
- P11 Telegram bot reachable
- P12 Telegram token available from local gitignored env

Fails:

- P2 streams binary check
- P3 host binary check
- P5 GitHub token loaded
- P8 Betterstack env + ingestion check
- P10 restart/resume gate check

Not-yet-ready:

- P9 backup tmux pane prewarmed

## Matrix

| Item | Result | Evidence | Notes |
| --- | --- | --- | --- |
| P1 Repo clean on main, latest | `PASS` | `## mono-thnc-6-4-main...origin/main` | Used an isolated main-based worktree per branch-hygiene directive. Branch name differs from `main`, but the worktree was clean and up to date after `pull --ff-only`. |
| P2 Streams binary present | `FAIL` | `fireline-streams not found` | Exact checklist command fails in the operator shell. Local binaries do exist at `target/release/fireline-streams` and `target/debug/fireline-streams`, but `fireline-streams --version` is not a valid clean pass on current main. |
| P3 Host binary present | `FAIL` | `fireline not found` | Exact checklist command fails in the operator shell. Local binaries do exist at `target/release/fireline` and `target/debug/fireline`, but `fireline --version` exits with `unexpected argument '--version'`. |
| P4 Anthropic key loaded | `PASS` | `10-char prefix present (redacted); len=108` | Operator shell has a non-empty Anthropic key. |
| P5 GitHub token loaded | `FAIL` | `len:0` | `GITHUB_TOKEN` is not loaded in the operator shell today. |
| P6 Demo assets present | `PASS` | `docs/demos/assets/README.md`, `agent.ts`, `reviewer.ts` | Frozen assets are present on current main. |
| P7 Local Docker image present + smoke | `PASS` | `fireline-host-quickstart:embedded-smoke b1a1ae209dc3` and `healthz:200` | A fresh container on alternate ports returned HTTP 200 from `/healthz`. |
| P8 OTel backend reachable (Betterstack) | `FAIL` | `betterstack_env:missing` | `deploy/observability/betterstack.env` is absent locally, so the checklist command cannot be run as written. No `OTEL_EXPORTER_OTLP_*` vars were already loaded in the shell either. |
| P9 Backup terminal prewarmed | `NOT-YET-READY` | `tmux:none` | No tmux session/pane was prewarmed during this dry-run. This remains a manual operator setup step. |
| P10 Step 3 restart/resume gate check | `FAIL` | `TypeError: Cannot read properties of undefined (reading 'toArray')` | Expected to fail until `mono-thnc.2.3` lands, but current-main failure occurs even earlier than the restart/resume beat: `docs/demos/scripts/replay-peer-to-peer.sh full` crashes before the kill-and-resume phase. |
| P11 Telegram bot reachable | `PASS` | `true` / `Jessica_fireline_bot` | This passed unexpectedly relative to the dispatch expectation. The local `deploy/telegram/bridge.env` token is valid right now. |
| P12 Telegram tokens sourced in operator env | `PASS` | `telegram_prefix_len:10` | `TELEGRAM_BOT_TOKEN` loads successfully from local `deploy/telegram/bridge.env`. |

## Commands and trimmed outputs

### P1

```bash
git status -sb
```

Output:

```text
## mono-thnc-6-4-main...origin/main
```

### P2

```bash
which fireline-streams
```

Output:

```text
fireline-streams not found
```

Additional local paths found:

```text
/Users/gnijor/gurdasnijor/fireline/target/release/fireline-streams
/Users/gnijor/gurdasnijor/fireline/target/debug/fireline-streams
```

### P3

```bash
which fireline
```

Output:

```text
fireline not found
```

Additional local paths found:

```text
/Users/gnijor/gurdasnijor/fireline/target/release/fireline
/Users/gnijor/gurdasnijor/fireline/target/debug/fireline
```

And:

```bash
/Users/gnijor/gurdasnijor/fireline/target/release/fireline --version
```

Output:

```text
error: unexpected argument '--version' found
```

### P4

```bash
echo $ANTHROPIC_API_KEY | head -c 10
```

Output:

```text
10-char prefix present (redacted)
```

### P5

```bash
echo $GITHUB_TOKEN | head -c 10
```

Output:

```text

```

### P6

```bash
ls docs/demos/assets/agent.ts docs/demos/assets/reviewer.ts docs/demos/assets/README.md
```

Output:

```text
docs/demos/assets/README.md
docs/demos/assets/agent.ts
docs/demos/assets/reviewer.ts
```

### P7

```bash
docker image ls fireline-host-quickstart:embedded-smoke
docker run -d --rm --name preflight-embedded-smoke -p 4542:4440 -p 7576:7474 fireline-host-quickstart:embedded-smoke
curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:4542/healthz
```

Output:

```text
fireline-host-quickstart:embedded-smoke b1a1ae209dc3
healthz:200
```

### P8

```bash
set -a; source deploy/observability/betterstack.env; set +a
```

Output:

```text
betterstack_env:missing
```

### P9

```bash
tmux list-sessions
```

Output:

```text
tmux:none
```

### P10

```bash
bash docs/demos/scripts/replay-peer-to-peer.sh full
```

Output:

```text
TypeError: Cannot read properties of undefined (reading 'toArray')
```

Observed failure context:

```text
file:///private/tmp/fireline-w24-preflight/[eval1]:37
    db.promptTurns.toArray
                   ^
```

This means the pre-flight gate is red today even before the expected
`mono-thnc.2.3` restart/resume failure mode.

### P11

```bash
source deploy/telegram/bridge.env
curl -s "https://api.telegram.org/bot$TELEGRAM_BOT_TOKEN/getMe" | jq -r '.ok,.result.username'
```

Output:

```text
true
Jessica_fireline_bot
```

### P12

```bash
echo $TELEGRAM_BOT_TOKEN | head -c 10
```

Output:

```text
10-char prefix present (redacted)
```

## Go / No-go for Rehearsal 1

Current status: `NO-GO for a fully-live run without downgrades`

Reasons:

- P10 is red, and it is red earlier than the expected session-resume gate
- P8 is red because Betterstack operator env is not present locally
- P5 is red because `GITHUB_TOKEN` is not loaded
- P2/P3 are red because the binary readiness commands in the script are stale relative to the current CLI/binary surface
- P9 is not staged yet

Safe live/pre-staged posture if rehearsal had to happen immediately:

- downgrade the restart/resume beat to PRE-STAGED
- treat Betterstack dashboard as unavailable until the env file is restored
- load `GITHUB_TOKEN` before rehearsal
- update the pre-flight checklist commands for binary readiness before showtime

