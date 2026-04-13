# FQA CLI Demo Capture

Status: replayable demo capture for FQA-1 CLI ergonomics smoke

Source of truth:
- [docs/reviews/fqa-cli-2026-04-12.md](./../reviews/fqa-cli-2026-04-12.md)
- [docs/demos/peer-to-peer-demo-capture.md](./peer-to-peer-demo-capture.md) for the replay/doc shape

## Prerequisites

- `fireline` and `fireline-streams` are already built locally.
  The replay script does not run `cargo`.
- `node`, `curl`, and `lsof` are installed.
- Ports `4440`, `7474`, `15440`, and `17474` are free, or the operator overrides `CLI_PORT`, `CLI_STREAMS_PORT`, `ALT_PORT`, and `ALT_STREAMS_PORT`.

Recommended env:

```bash
export FIRELINE_BIN="$PWD/target/debug/fireline"
export FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams"
```

## Honest Scope

This replay keeps the reviewed FQA-1 scenario set:
- boot + teardown on the default ports
- bad path handling
- known agent install id
- unknown agent install id
- bare `agents` invocation
- global help output
- boot + teardown on alternate ports

Current replay result on this checkout:
- `run` boot/teardown is healthy on both the default and alternate ports.
- on the default `:7474` path, the CLI may reuse an already-running local `fireline-streams` instead of spawning and tearing down a new one. That is current surfaced behavior, not a replay bug.
- bad-path handling is healthy.
- `agents add pi-acp` succeeds, but only via the installed stub side effect; it prints no stdout today.
- `agents add does-not-exist` still exits `0` and prints nothing today.
- bare `fireline agents` still exits `0` and prints nothing today.
- `--help` is healthier than the original 2026-04-12 review because it now advertises `agents` and uses the shipped minimal spec example.

Surface note:
- the replay script only drives the advertised CLI surface via `node packages/fireline/dist/cli.js ...`
- there is no raw ACP driver or internal host binary in this capture path

## Quick Replay

One-shot replay:

1. `FIRELINE_BIN="$PWD/target/debug/fireline" FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams" ./docs/demos/scripts/replay-fqa-cli.sh`

Expected stdout excerpt:

```json
{
  "summaryPath": "/.../.tmp/fqa-cli-demo/latest/summary.json",
  "bootExcerpt": "durable-streams ready at http://127.0.0.1:7474/v1/stream ... ✓ fireline ready ...",
  "knownAgentInstalledPath": "/Users/.../Library/Application Support/fireline/agents/bin/pi-acp",
  "unknownAgentExitCode": 0,
  "noArgsAgentsVerdict": "fail",
  "helpVerdict": "pass",
  "altPortsStateUrl": "http://127.0.0.1:17474/v1/stream/..."
}
```

Artifacts written by the script:
- `.tmp/fqa-cli-demo/latest/summary.json`
- `.tmp/fqa-cli-demo/latest/logs/`

## Manual Replay

This is the same path, broken out into operator-visible commands a user could type after reading [packages/fireline/README.md](/Users/gnijor/gurdasnijor/fireline/packages/fireline/README.md:1).

1. `export FIRELINE_BIN="$PWD/target/debug/fireline"`

Expected result:
- no stdout

2. `export FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams"`

Expected result:
- no stdout

3. `node packages/fireline/dist/cli.js --help`

Expected stdout excerpt:

```text
fireline — run specs locally, build hosted images, deploy them, or install ACP agents
...
fireline agents <command> [args]
...
fireline run packages/fireline/test-fixtures/minimal-spec.ts
```

4. `node packages/fireline/dist/cli.js run packages/fireline/test-fixtures/minimal-spec.ts`

Expected stdout excerpt:

```text
durable-streams ready at http://127.0.0.1:7474/v1/stream

  ✓ fireline ready

    ACP:       ws://127.0.0.1:...
    state:     http://127.0.0.1:7474/v1/stream/fireline-state-runtime-...
```

Pass marker:
- press `Ctrl+C`; the command exits with `130`

5. `node packages/fireline/dist/cli.js run /tmp/does-not-exist.ts`

Expected stdout excerpt:

```text
fireline: Cannot find module '/tmp/does-not-exist.ts' imported from /tmp/
```

6. `node packages/fireline/dist/cli.js agents add pi-acp`

Expected result:
- exit code `0`
- no stdout today
- installed stub appears at `~/Library/Application Support/fireline/agents/bin/pi-acp`

7. `node packages/fireline/dist/cli.js agents add does-not-exist`

Expected result:
- exit code `0` today
- no stdout today
- no installed stub for `does-not-exist`

8. `node packages/fireline/dist/cli.js agents`

Expected result:
- exit code `0` today
- no usage text today

9. `node packages/fireline/dist/cli.js run packages/fireline/test-fixtures/minimal-spec.ts --port 15440 --streams-port 17474`

Expected stdout excerpt:

```text
durable-streams ready at http://127.0.0.1:17474/v1/stream
...
state:     http://127.0.0.1:17474/v1/stream/fireline-state-runtime-...
```

Pass marker:
- press `Ctrl+C`; the command exits with `130`

10. `./docs/demos/scripts/replay-fqa-cli.sh`

Expected stdout excerpt:

```json
{
  "summaryPath": "/.../.tmp/fqa-cli-demo/latest/summary.json",
  "unknownAgentExitCode": 0,
  "noArgsAgentsVerdict": "fail",
  "helpVerdict": "pass"
}
```

11. `cat .tmp/fqa-cli-demo/latest/summary.json`

Expected JSON excerpts:

```json
{
  "scenarioResults": [
    { "name": "boot-default", "verdict": "pass", "exitCode": 130 },
    { "name": "bad-path", "verdict": "pass", "exitCode": 1 },
    { "name": "known-agent-id", "verdict": "pass", "exitCode": 0 },
    { "name": "unknown-agent-id", "verdict": "fail", "exitCode": 0 },
    { "name": "agents-no-args", "verdict": "fail", "exitCode": 0 },
    { "name": "help", "verdict": "pass", "exitCode": 0 },
    { "name": "boot-alt-ports", "verdict": "pass", "exitCode": 130 }
  ],
  "notes": {
    "knownAgentInstallMayBeSilent": true,
    "unknownAgentLookupSurfacedAnError": false,
    "bareAgentsInvocationPrintedUsage": false
  }
}
```

Pass markers:
- both boot scenarios are `pass`
- `bad-path` is `pass`
- `help` is `pass`
- the two known ergonomics gaps are still called out explicitly as `fail`, not hidden
- if `boot-default.reusedExistingStreams` is `true`, the default-port run reused the local `:7474` service intentionally

## Expected Outputs By Surface

### Operator stdout

Expected success markers:
- the replay prints a `summaryPath`
- the boot excerpt includes `✓ fireline ready`
- the alternate-port state URL uses `:17474`

### CLI behavior snapshots

Expected healthy surfaces:
- `run packages/fireline/test-fixtures/minimal-spec.ts` reaches ready and exits `130` after the scripted `SIGINT`
- `run /tmp/does-not-exist.ts` exits `1` and names the missing path directly
- `--help` advertises `run`, `build`, `deploy`, and `agents`

Expected rough edges:
- `agents add pi-acp` installs the stub at `~/Library/Application Support/fireline/agents/bin/pi-acp` but prints no stdout
- `agents add does-not-exist` exits `0` and surfaces no lookup error
- bare `fireline agents` exits `0` and prints no usage text

### Installed agent stub

Expected installed file contents for the known id:

```bash
#!/usr/bin/env bash
set -euo pipefail
exec npx -y 'pi-acp@0.0.25' "$@"
```

## Pass / Fail Markers

| Step | Expected verdict | What counts as pass | What counts as fail |
| --- | --- | --- | --- |
| Default-port boot + teardown | Pass | ready banner prints; `SIGINT` exit is `130`; control-plane listener disappears after teardown; pre-existing `:7474` may be reused | ready banner never appears or control-plane listener remains |
| Bad path | Pass | exit code `1` and missing path named directly | stack trace or unrelated error |
| Known agent id | Pass with UX caveat | stub exists at the install path after `agents add pi-acp` | command exits non-zero or stub missing |
| Unknown agent id | Fail today | n/a | exits `0` with no lookup error |
| Bare `agents` | Fail today | n/a | exits `0` with no usage text |
| Global help | Pass | help text shows `agents` and the minimal spec example | help omits the current surface |
| Alternate-port boot + teardown | Pass | ready banner prints on `17474`; listeners disappear after teardown | alternate ports never become healthy |

## Rough Edges And Workarounds

- **Rough edge:** the known-id install path is effectively silent.
  **Workaround:** verify the stub file in `~/Library/Application Support/fireline/agents/bin/`.
- **Rough edge:** if `:7474` already has a local `fireline-streams`, the default-port run reuses it instead of tearing it down.
  **Workaround:** use the alternate-port run for a fully self-contained boot/teardown proof.
- **Rough edge:** unknown ids still do not surface a CLI error.
  **Workaround:** do not use this path live in narration; keep the failure in the summary JSON as the honest QA result.
- **Rough edge:** bare `fireline agents` does not print usage today.
  **Workaround:** use `fireline agents --help` off-stage if you need the subcommand help text during rehearsal.

## Recorded Replay

Recorded replay: [pending — captured during rehearsal]

Intended filename:
- `docs/demos/assets/recordings/fqa-1-cli.mp4`
