# FQA Approval Demo Capture

Status: replayable demo capture for FQA-4 approval-gated session using advertised surfaces

Source of truth:
- [docs/reviews/fqa-approval-session-2026-04-12.md](./../reviews/fqa-approval-session-2026-04-12.md)
- [examples/approval-workflow/index.ts](/Users/gnijor/gurdasnijor/fireline/examples/approval-workflow/index.ts:1)
- [examples/approval-workflow/README.md](/Users/gnijor/gurdasnijor/fireline/examples/approval-workflow/README.md:1)
- [docs/demos/peer-to-peer-demo-capture.md](./peer-to-peer-demo-capture.md) for the replay/doc shape

## Prerequisites

- `fireline`, `fireline-streams`, and `fireline-testy-fs` are already built locally.
  The replay does not run `cargo`.
- `node` is installed.
- Port `4540` and port `7574` are free, or the operator overrides `CONTROL_PORT` and `STREAMS_PORT`.

Recommended env:

```bash
export FIRELINE_BIN="$PWD/target/debug/fireline"
export FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams"
export FQA_APPROVAL_AGENT_COMMAND="$PWD/target/debug/fireline-testy-fs"
```

## Honest Scope

This capture is intentionally narrower than the original FQA-4 QA harness. It uses only the surfaced paths we advertise today:
- `node packages/fireline/dist/cli.js run ...` to boot Fireline with a public harness spec
- `@fireline/client` public APIs in [docs/demos/scripts/replay-fqa-approval.mjs](/Users/gnijor/gurdasnijor/fireline/docs/demos/scripts/replay-fqa-approval.mjs:1):
  `connectAcp(...)`, `fireline.db(...)`, and `appendApprovalResolved(...)`

What this public-surface capture proves today:
- approval allow path works end to end
- approval deny path works end to end
- the live approval gate still uses the prompt-level fallback wording for `approve({ scope: 'tool_calls' })`
- denied approvals still surface the generic ACP error `Internal error`

What this public-surface capture does **not** reproduce today:
- the original FQA-4 crash + `kill -9` + restart + `session/load` leg

Truthful finding:
- that crash/restart leg is still not reproducible from the advertised CLI/public-client story alone. It needs an internal host/process-control path outside the surfaced product workflow. The source QA review remains the reference for that failure mode.
- tracked follow-up: `mono-thnc.9.1` — public-surface crash/restart path for FQA-4 parity

## Quick Replay

One-shot replay:

1. `FIRELINE_BIN="$PWD/target/debug/fireline" FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams" FQA_APPROVAL_AGENT_COMMAND="$PWD/target/debug/fireline-testy-fs" node docs/demos/scripts/replay-fqa-approval.mjs`

Expected stdout excerpt:

```json
{
  "summaryPath": "/.../.tmp/fqa-approval-demo/latest/summary.json",
  "acpUrl": "ws://127.0.0.1:...",
  "stateUrl": "http://127.0.0.1:7574/v1/stream/fqa-approval-public",
  "allowVerdict": "pass",
  "denyVerdict": "pass",
  "promptLevelFallback": true,
  "publicSurfaceCoversCrashRestartSessionLoad": false
}
```

Artifacts written by the script:
- `.tmp/fqa-approval-demo/latest/summary.json`
- `.tmp/fqa-approval-demo/latest/logs/fireline-cli.log`

## Manual Replay

This is the same path, broken out into operator-visible commands a user could realistically type after reading [examples/approval-workflow/README.md](/Users/gnijor/gurdasnijor/fireline/examples/approval-workflow/README.md:1).

1. `export FIRELINE_BIN="$PWD/target/debug/fireline"`

Expected result:
- no stdout

2. `export FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams"`

Expected result:
- no stdout

3. `export FQA_APPROVAL_AGENT_COMMAND="$PWD/target/debug/fireline-testy-fs"`

Expected result:
- no stdout

4. `node packages/fireline/dist/cli.js run docs/demos/scripts/fqa-approval-harness.ts --port 4540 --streams-port 7574 --state-stream fqa-approval-public`

Expected stdout excerpt:

```text
durable-streams ready at http://127.0.0.1:7574/v1/stream

  ✓ fireline ready

    ACP:       ws://127.0.0.1:...
    state:     http://127.0.0.1:7574/v1/stream/fqa-approval-public
```

Operator note:
- leave this terminal running

5. `node docs/demos/scripts/replay-fqa-approval.mjs driver-only --acp-url ws://127.0.0.1:<printed-port>/acp --state-url http://127.0.0.1:7574/v1/stream/fqa-approval-public`

Expected stdout excerpt:

```json
{
  "summaryPath": "/.../.tmp/fqa-approval-demo/latest/summary.json",
  "allowVerdict": "pass",
  "denyVerdict": "pass",
  "promptLevelFallback": true,
  "publicSurfaceCoversCrashRestartSessionLoad": false
}
```

6. `cat .tmp/fqa-approval-demo/latest/summary.json`

Expected JSON excerpts:

```json
{
  "scenarioResults": [
    {
      "name": "approval-allow",
      "verdict": "pass",
      "permissionTitle": "approval fallback: prompt-level gate until tool-call interception lands"
    },
    {
      "name": "approval-deny",
      "verdict": "pass",
      "promptError": "Internal error"
    }
  ],
  "limitations": {
    "promptLevelFallback": true,
    "deniedPathReturnedGenericInternalError": true,
    "publicSurfaceCoversCrashRestartSessionLoad": false
  }
}
```

7. Return to the first terminal and press `Ctrl+C`

Expected result:
- the CLI exits with `130`

## Expected Outputs By Surface

### Operator stdout

Expected success markers:
- the surfaced CLI prints the ready banner with ACP and state URLs
- the public client replay prints a `summaryPath`
- `allowVerdict` is `pass`
- `denyVerdict` is `pass`

### Public client API evidence

Expected surfaced APIs in use:
- `connectAcp(...)` opens the ACP client connection
- `fireline.db({ stateStreamUrl })` materializes the state stream
- `appendApprovalResolved(...)` resolves the pending approval from outside the agent run

Expected allow-path evidence:
- a pending permission row appears for the prompt
- the permission title includes `approval fallback: prompt-level gate until tool-call interception lands`
- after approval, one chunk contains `ok:/workspace/fqa-approved.txt`

Expected deny-path evidence:
- a pending permission row appears for the prompt
- after denial, no success chunk for `/workspace/fqa-denied.txt` appears
- the prompt returns `Internal error`

### Original FQA-4 gap

Expected truthful limitation:
- `publicSurfaceCoversCrashRestartSessionLoad` is `false`
- the doc does not pretend the original crash + restart + `session/load` QA leg is available from the advertised operator path today

## Pass / Fail Markers

| Step | Expected verdict | What counts as pass | What counts as fail |
| --- | --- | --- | --- |
| Surfaced CLI boot | Pass | ready banner prints ACP/state URLs | boot never reaches ready |
| Approval allow path via public client API | Pass | permission appears, approval resolves, chunk contains `ok:/workspace/fqa-approved.txt` | no permission or no success chunk |
| Approval deny path via public client API | Pass with UX caveat | permission appears, deny returns `Internal error`, no success chunk is emitted | denied write still succeeds |
| `approve({ scope: 'tool_calls' })` literal tool-call interception | Fail today | n/a | permission title still says fallback |
| Crash/restart/`session/load` from advertised surface | Not reproducible today | n/a | no surfaced operator path exists for that leg |

## Rough Edges And Workarounds

- **Rough edge:** `approve({ scope: 'tool_calls' })` still materializes as the prompt-level fallback path.
  **Workaround:** narrate it honestly as the current live approval surface.
- **Rough edge:** denied approvals still return the generic ACP error `Internal error`.
  **Workaround:** keep that caveat in the operator notes; do not overclaim the denial UX.
- **Rough edge:** the original FQA crash/restart/session-load leg is not reproducible from the advertised CLI/public-client story.
  **Workaround:** treat that as QA evidence only and point to [docs/reviews/fqa-approval-session-2026-04-12.md](/Users/gnijor/gurdasnijor/fireline/docs/reviews/fqa-approval-session-2026-04-12.md:1) for the current failure.

## Recorded Replay

Recorded replay: [pending — captured during rehearsal]

Intended filename:
- `docs/demos/assets/recordings/fqa-4-approval.mp4`
