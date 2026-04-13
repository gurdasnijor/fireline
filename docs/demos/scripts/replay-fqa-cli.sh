#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/../../.." && pwd)
ARTIFACT_ROOT="${ARTIFACT_ROOT:-$REPO_ROOT/.tmp/fqa-cli-demo}"
RUN_DIR="${RUN_DIR:-$ARTIFACT_ROOT/latest}"
LOG_DIR="$RUN_DIR/logs"

CLI_PORT="${CLI_PORT:-4440}"
CLI_STREAMS_PORT="${CLI_STREAMS_PORT:-7474}"
ALT_PORT="${ALT_PORT:-15440}"
ALT_STREAMS_PORT="${ALT_STREAMS_PORT:-17474}"
KNOWN_AGENT_ID="${KNOWN_AGENT_ID:-pi-acp}"
UNKNOWN_AGENT_ID="${UNKNOWN_AGENT_ID:-does-not-exist}"

FIRELINE_BIN="${FIRELINE_BIN:-$REPO_ROOT/target/debug/fireline}"
FIRELINE_STREAMS_BIN="${FIRELINE_STREAMS_BIN:-$REPO_ROOT/target/debug/fireline-streams}"
CLI_ENTRY="${CLI_ENTRY:-$REPO_ROOT/packages/fireline/dist/cli.js}"
MINIMAL_SPEC="${MINIMAL_SPEC:-$REPO_ROOT/packages/fireline/test-fixtures/minimal-spec.ts}"
INSTALLED_AGENT_PATH="${INSTALLED_AGENT_PATH:-$HOME/Library/Application Support/fireline/agents/bin/$KNOWN_AGENT_ID}"
UNKNOWN_AGENT_PATH="${UNKNOWN_AGENT_PATH:-$HOME/Library/Application Support/fireline/agents/bin/$UNKNOWN_AGENT_ID}"

log() {
  printf '[fqa-cli] %s\n' "$*"
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'missing required command: %s\n' "$1" >&2
    exit 1
  fi
}

require_file() {
  if [[ ! -f "$1" ]]; then
    printf 'missing required file: %s\n' "$1" >&2
    exit 1
  fi
}

require_executable() {
  if [[ ! -x "$1" ]]; then
    printf 'missing executable: %s\n' "$1" >&2
    exit 1
  fi
}

setup_dirs() {
  mkdir -p "$LOG_DIR"
}

run_capture() {
  local name="$1"
  shift

  local log_file="$LOG_DIR/$name.log"
  local exit_file="$LOG_DIR/$name.exit"

  set +e
  "$@" >"$log_file" 2>&1
  local exit_code=$?
  set -e

  printf '%s\n' "$exit_code" >"$exit_file"
}

wait_for_pattern() {
  local pattern="$1"
  local file="$2"
  local label="$3"

  for _ in $(seq 1 200); do
    if grep -q "$pattern" "$file" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done

  printf 'timed out waiting for %s in %s\n' "$label" "$file" >&2
  return 1
}

capture_boot_scenario() {
  local name="$1"
  local port="$2"
  local streams_port="$3"
  local mode="$4"

  local log_file="$LOG_DIR/$name.log"
  local exit_file="$LOG_DIR/$name.exit"
  local pre_listener_file="$LOG_DIR/$name.pre-listeners.txt"
  local post_listener_file="$LOG_DIR/$name.post-listeners.txt"
  local pid_file="$LOG_DIR/$name.pid"

  : >"$pre_listener_file"
  : >"$post_listener_file"

  local cmd=(
    env
    FIRELINE_BIN="$FIRELINE_BIN"
    FIRELINE_STREAMS_BIN="$FIRELINE_STREAMS_BIN"
    node
    "$CLI_ENTRY"
    run
    "$MINIMAL_SPEC"
  )

  if [[ "$mode" == "alt" ]]; then
    cmd+=(
      --port "$port"
      --streams-port "$streams_port"
    )
  fi

  "${cmd[@]}" >"$log_file" 2>&1 &
  local pid=$!
  printf '%s\n' "$pid" >"$pid_file"

  if ! wait_for_pattern 'fireline ready' "$log_file" "$name readiness"; then
    set +e
    kill "$pid" >/dev/null 2>&1 || true
    wait "$pid" >/dev/null 2>&1 || true
    set -e
    printf '1\n' >"$exit_file"
    return
  fi

  {
    printf 'control-plane (%s)\n' "$port"
    lsof -nP -iTCP:"$port" -sTCP:LISTEN 2>/dev/null || true
    printf '\n'
    printf 'durable-streams (%s)\n' "$streams_port"
    lsof -nP -iTCP:"$streams_port" -sTCP:LISTEN 2>/dev/null || true
  } >"$pre_listener_file"

  set +e
  kill -INT "$pid" >/dev/null 2>&1 || true
  wait "$pid"
  local exit_code=$?
  set -e
  printf '%s\n' "$exit_code" >"$exit_file"

  {
    printf 'control-plane (%s)\n' "$port"
    lsof -nP -iTCP:"$port" -sTCP:LISTEN 2>/dev/null || true
    printf '\n'
    printf 'durable-streams (%s)\n' "$streams_port"
    lsof -nP -iTCP:"$streams_port" -sTCP:LISTEN 2>/dev/null || true
  } >"$post_listener_file"
}

build_summary() {
  export RUN_DIR LOG_DIR CLI_PORT CLI_STREAMS_PORT ALT_PORT ALT_STREAMS_PORT
  export KNOWN_AGENT_ID UNKNOWN_AGENT_ID INSTALLED_AGENT_PATH UNKNOWN_AGENT_PATH
  node --input-type=module <<'EOF'
import fs from 'node:fs/promises'
import path from 'node:path'

const runDir = process.env.RUN_DIR
const logDir = process.env.LOG_DIR

function readText(file) {
  return fs.readFile(path.join(logDir, file), 'utf8')
}

function excerpt(text, pattern, fallbackLines = 12) {
  if (!text.trim()) return ''
  if (!pattern) {
    return text.trim().split('\n').slice(0, fallbackLines).join('\n')
  }
  const lines = text.split('\n')
  const index = lines.findIndex((line) => line.includes(pattern))
  if (index === -1) {
    return lines.slice(0, fallbackLines).join('\n').trim()
  }
  const start = Math.max(index - 2, 0)
  const end = Math.min(index + 6, lines.length)
  return lines.slice(start, end).join('\n').trim()
}

function parseField(text, label) {
  const match = text.match(new RegExp(`${label}:\\s+(\\S+)`))
  return match?.[1] ?? null
}

function truthy(text) {
  return Boolean(text && text.trim())
}

async function fileExists(filePath) {
  try {
    await fs.access(filePath)
    return true
  } catch {
    return false
  }
}

async function readInstalledAgent(filePath) {
  if (!(await fileExists(filePath))) return null
  return (await fs.readFile(filePath, 'utf8')).trim()
}

const [
  bootLog,
  bootExit,
  bootPreListeners,
  bootPostListeners,
  badPathLog,
  badPathExit,
  knownLog,
  knownExit,
  unknownLog,
  unknownExit,
  noArgsLog,
  noArgsExit,
  helpLog,
  helpExit,
  altLog,
  altExit,
  altPreListeners,
  altPostListeners,
] = await Promise.all([
  readText('boot-default.log'),
  readText('boot-default.exit'),
  readText('boot-default.pre-listeners.txt'),
  readText('boot-default.post-listeners.txt'),
  readText('bad-path.log'),
  readText('bad-path.exit'),
  readText('agents-known.log'),
  readText('agents-known.exit'),
  readText('agents-unknown.log'),
  readText('agents-unknown.exit'),
  readText('agents-no-args.log'),
  readText('agents-no-args.exit'),
  readText('help.log'),
  readText('help.exit'),
  readText('boot-alt.log'),
  readText('boot-alt.exit'),
  readText('boot-alt.pre-listeners.txt'),
  readText('boot-alt.post-listeners.txt'),
])

const installedAgentPath = process.env.INSTALLED_AGENT_PATH
const unknownAgentPath = process.env.UNKNOWN_AGENT_PATH
const installedAgentContents = await readInstalledAgent(installedAgentPath)
const unknownAgentInstalled = await fileExists(unknownAgentPath)
const defaultReusedStreams = bootLog.includes('reusing fireline-streams')
const defaultControlPlaneGone = !bootPostListeners.includes(`TCP 127.0.0.1:${process.env.CLI_PORT} (LISTEN)`)
const altControlPlaneGone = !altPostListeners.includes(`TCP 127.0.0.1:${process.env.ALT_PORT} (LISTEN)`)
const altStreamsGone = !altPostListeners.includes(`TCP 127.0.0.1:${process.env.ALT_STREAMS_PORT} (LISTEN)`)

const bootDefault = {
  name: 'boot-default',
  verdict:
    Number(bootExit.trim()) === 130 &&
    truthy(parseField(bootLog, 'ACP')) &&
    truthy(parseField(bootLog, 'state')) &&
    defaultControlPlaneGone &&
    (defaultReusedStreams || !bootPostListeners.includes(`TCP 127.0.0.1:${process.env.CLI_STREAMS_PORT} (LISTEN)`))
      ? 'pass'
      : 'fail',
  exitCode: Number(bootExit.trim()),
  acpUrl: parseField(bootLog, 'ACP'),
  stateUrl: parseField(bootLog, 'state'),
  excerpt: excerpt(bootLog, 'fireline ready'),
  preListeners: bootPreListeners.trim(),
  postListeners: bootPostListeners.trim(),
  reusedExistingStreams: defaultReusedStreams,
}

const badPath = {
  name: 'bad-path',
  verdict:
    Number(badPathExit.trim()) === 1 &&
    badPathLog.includes('/tmp/does-not-exist.ts')
      ? 'pass'
      : 'fail',
  exitCode: Number(badPathExit.trim()),
  excerpt: excerpt(badPathLog, 'Cannot find module'),
}

const knownAgent = {
  name: 'known-agent-id',
  verdict:
    Number(knownExit.trim()) === 0 &&
    truthy(installedAgentContents) &&
    installedAgentContents.includes(process.env.KNOWN_AGENT_ID)
      ? 'pass'
      : 'fail',
  exitCode: Number(knownExit.trim()),
  excerpt: excerpt(knownLog, null),
  installedPath: installedAgentPath,
  installedContents: installedAgentContents,
}

const unknownAgent = {
  name: 'unknown-agent-id',
  verdict:
    Number(unknownExit.trim()) !== 0 &&
    (unknownLog.includes(process.env.UNKNOWN_AGENT_ID) || !unknownAgentInstalled)
      ? 'pass'
      : 'fail',
  exitCode: Number(unknownExit.trim()),
  excerpt: excerpt(unknownLog, null),
  installedPath: unknownAgentPath,
  installed: unknownAgentInstalled,
}

const noArgsAgents = {
  name: 'agents-no-args',
  verdict:
    noArgsLog.includes('Usage:') || noArgsLog.includes('fireline agents')
      ? 'pass'
      : 'fail',
  exitCode: Number(noArgsExit.trim()),
  excerpt: excerpt(noArgsLog, null),
}

const help = {
  name: 'help',
  verdict:
    Number(helpExit.trim()) === 0 &&
    helpLog.includes('fireline agents add pi-acp') &&
    helpLog.includes('packages/fireline/test-fixtures/minimal-spec.ts')
      ? 'pass'
      : 'fail',
  exitCode: Number(helpExit.trim()),
  excerpt: excerpt(helpLog, 'Usage:'),
}

const bootAlt = {
  name: 'boot-alt-ports',
  verdict:
    Number(altExit.trim()) === 130 &&
    (parseField(altLog, 'state') ?? '').includes(`:${process.env.ALT_STREAMS_PORT}/`) &&
    altControlPlaneGone &&
    altStreamsGone
      ? 'pass'
      : 'fail',
  exitCode: Number(altExit.trim()),
  acpUrl: parseField(altLog, 'ACP'),
  stateUrl: parseField(altLog, 'state'),
  excerpt: excerpt(altLog, 'fireline ready'),
  preListeners: altPreListeners.trim(),
  postListeners: altPostListeners.trim(),
}

const summary = {
  sourceReview: 'docs/reviews/fqa-cli-2026-04-12.md',
  summaryPath: path.join(runDir, 'summary.json'),
  executedAt: new Date().toISOString(),
  scenarioResults: [
    bootDefault,
    badPath,
    knownAgent,
    unknownAgent,
    noArgsAgents,
    help,
    bootAlt,
  ],
  notes: {
    knownAgentInstallMayBeSilent: !truthy(knownLog),
    unknownAgentLookupSurfacedAnError: Number(unknownExit.trim()) !== 0 || truthy(unknownLog),
    bareAgentsInvocationPrintedUsage: noArgsLog.includes('Usage:') || noArgsLog.includes('fireline agents'),
  },
}

await fs.writeFile(summary.summaryPath, `${JSON.stringify(summary, null, 2)}\n`)

console.log(JSON.stringify({
  summaryPath: summary.summaryPath,
  bootExcerpt: bootDefault.excerpt,
  knownAgentInstalledPath: knownAgent.installedPath,
  unknownAgentExitCode: unknownAgent.exitCode,
  noArgsAgentsVerdict: noArgsAgents.verdict,
  helpVerdict: help.verdict,
  altPortsStateUrl: bootAlt.stateUrl,
}, null, 2))
EOF
}

main() {
  require_cmd curl
  require_cmd lsof
  require_cmd node
  require_file "$CLI_ENTRY"
  require_file "$MINIMAL_SPEC"
  require_executable "$FIRELINE_BIN"
  require_executable "$FIRELINE_STREAMS_BIN"
  setup_dirs

  log "booting CLI on default ports"
  capture_boot_scenario "boot-default" "$CLI_PORT" "$CLI_STREAMS_PORT" "default"

  log "capturing bad-path failure"
  run_capture "bad-path" node "$CLI_ENTRY" run /tmp/does-not-exist.ts

  log "capturing known agent install path"
  run_capture "agents-known" node "$CLI_ENTRY" agents add "$KNOWN_AGENT_ID"

  log "capturing unknown agent install path"
  run_capture "agents-unknown" node "$CLI_ENTRY" agents add "$UNKNOWN_AGENT_ID"

  log "capturing bare agents invocation"
  run_capture "agents-no-args" node "$CLI_ENTRY" agents

  log "capturing global help"
  run_capture "help" node "$CLI_ENTRY" --help

  log "booting CLI on alternate ports"
  capture_boot_scenario "boot-alt" "$ALT_PORT" "$ALT_STREAMS_PORT" "alt"

  build_summary
}

main "$@"
