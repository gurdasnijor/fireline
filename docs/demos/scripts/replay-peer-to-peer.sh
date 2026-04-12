#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/../../.." && pwd)
ARTIFACT_ROOT="${ARTIFACT_ROOT:-$REPO_ROOT/.tmp/peer-to-peer-demo}"
RUN_DIR="${RUN_DIR:-$ARTIFACT_ROOT/latest}"
PID_DIR="$RUN_DIR/pids"
LOG_DIR="$RUN_DIR/logs"

STREAMS_PORT="${STREAMS_PORT:-7474}"
CONTROL_A_PORT="${CONTROL_A_PORT:-4440}"
CONTROL_B_PORT="${CONTROL_B_PORT:-5440}"
STATE_STREAM_A="${STATE_STREAM_A:-peer-demo-a}"
STATE_STREAM_B="${STATE_STREAM_B:-peer-demo-b}"
AGENT_A_NAME="${AGENT_A_NAME:-agent-a}"
AGENT_B_NAME="${AGENT_B_NAME:-agent-b}"
PROMPT_MESSAGE="${PROMPT_MESSAGE:-hello across shared stream}"

FIRELINE_BIN="${FIRELINE_BIN:-$REPO_ROOT/target/debug/fireline}"
FIRELINE_STREAMS_BIN="${FIRELINE_STREAMS_BIN:-$REPO_ROOT/target/debug/fireline-streams}"
AGENT_BIN="${AGENT_BIN:-$REPO_ROOT/target/debug/fireline-testy}"

MODE="${1:-full}"

log() {
  printf '[peer-demo] %s\n' "$*"
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'missing required command: %s\n' "$1" >&2
    exit 1
  fi
}

require_executable() {
  if [[ ! -x "$1" ]]; then
    printf 'missing executable: %s\n' "$1" >&2
    exit 1
  fi
}

wait_for_http() {
  local url="$1"
  local label="$2"
  for _ in $(seq 1 100); do
    if curl -fsS "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  printf 'timed out waiting for %s at %s\n' "$label" "$url" >&2
  exit 1
}

pid_is_live() {
  local pid_file="$1"
  [[ -f "$pid_file" ]] || return 1
  local pid
  pid=$(cat "$pid_file")
  kill -0 "$pid" >/dev/null 2>&1
}

start_process() {
  local label="$1"
  local pid_file="$2"
  local log_file="$3"
  shift 3

  if pid_is_live "$pid_file"; then
    log "$label already running (pid $(cat "$pid_file"))"
    return 0
  fi

  log "starting $label"
  "$@" >"$log_file" 2>&1 &
  echo $! >"$pid_file"
}

setup_dirs() {
  mkdir -p "$PID_DIR" "$LOG_DIR"
}

setup() {
  require_cmd curl
  require_cmd node
  require_executable "$FIRELINE_BIN"
  require_executable "$FIRELINE_STREAMS_BIN"
  require_executable "$AGENT_BIN"

  setup_dirs

  start_process \
    "fireline-streams" \
    "$PID_DIR/fireline-streams.pid" \
    "$LOG_DIR/fireline-streams.log" \
    env PORT="$STREAMS_PORT" "$FIRELINE_STREAMS_BIN"
  wait_for_http "http://127.0.0.1:$STREAMS_PORT/healthz" "fireline-streams"

  start_process \
    "control-plane-a" \
    "$PID_DIR/control-plane-a.pid" \
    "$LOG_DIR/control-plane-a.log" \
    "$FIRELINE_BIN" \
    --control-plane \
    --port "$CONTROL_A_PORT" \
    --durable-streams-url "http://127.0.0.1:$STREAMS_PORT/v1/stream"
  wait_for_http "http://127.0.0.1:$CONTROL_A_PORT/healthz" "control-plane-a"

  start_process \
    "control-plane-b" \
    "$PID_DIR/control-plane-b.pid" \
    "$LOG_DIR/control-plane-b.log" \
    "$FIRELINE_BIN" \
    --control-plane \
    --port "$CONTROL_B_PORT" \
    --durable-streams-url "http://127.0.0.1:$STREAMS_PORT/v1/stream"
  wait_for_http "http://127.0.0.1:$CONTROL_B_PORT/healthz" "control-plane-b"
}

driver() {
  setup_dirs
  export REPO_ROOT RUN_DIR STREAMS_PORT CONTROL_A_PORT CONTROL_B_PORT
  export STATE_STREAM_A STATE_STREAM_B AGENT_A_NAME AGENT_B_NAME PROMPT_MESSAGE AGENT_BIN
  node --input-type=module <<'EOF'
import fs from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const rawConsoleLog = console.log.bind(console)
console.log = (...args) => {
  if (typeof args[0] === 'string' && args[0].startsWith('[StreamDB]')) {
    return
  }
  rawConsoleLog(...args)
}

const repoRoot = process.env.REPO_ROOT
const runDir = process.env.RUN_DIR
const serverA = `http://127.0.0.1:${process.env.CONTROL_A_PORT}`
const serverB = `http://127.0.0.1:${process.env.CONTROL_B_PORT}`
const agentBin = process.env.AGENT_BIN
const stateStreamA = process.env.STATE_STREAM_A
const stateStreamB = process.env.STATE_STREAM_B
const agentAName = process.env.AGENT_A_NAME
const agentBName = process.env.AGENT_B_NAME
const promptMessage = process.env.PROMPT_MESSAGE

const firelineMod = await import(pathToFileURL(path.join(repoRoot, 'packages/client/dist/index.js')).href)
const middlewareMod = await import(pathToFileURL(path.join(repoRoot, 'packages/client/dist/middleware.js')).href)

const fireline = firelineMod.default
const { agent, compose, middleware, sandbox } = firelineMod
const { peer } = middlewareMod

function toolCall(tool, params = {}) {
  return JSON.stringify({ command: 'call_tool', server: 'fireline-peer', tool, params })
}

function readSessionText(db, sessionId) {
  const turnIds = new Set(
    db.promptTurns.toArray
      .filter((turn) => turn.sessionId === sessionId)
      .map((turn) => turn.promptTurnId),
  )
  return db.chunks.toArray
    .filter((chunk) => turnIds.has(chunk.promptTurnId))
    .map((chunk) => chunk.content)
    .join('\n')
}

function waitForRows(collection, predicate, timeoutMs) {
  const snapshot = () => [...collection.toArray]
  if (predicate(snapshot())) return Promise.resolve(snapshot())
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      subscription.unsubscribe()
      reject(new Error(`timed out after ${timeoutMs}ms`))
    }, timeoutMs)
    const subscription = collection.subscribeChanges(() => {
      const rows = snapshot()
      if (!predicate(rows)) return
      clearTimeout(timeout)
      subscription.unsubscribe()
      resolve(rows)
    })
  })
}

function latestSessionRows(db, sessionId) {
  const promptTurns = db.promptTurns.toArray.filter((turn) => turn.sessionId === sessionId)
  const promptTurnIds = new Set(promptTurns.map((turn) => turn.promptTurnId))
  return {
    sessionId,
    sessions: db.sessions.toArray.filter((session) => session.sessionId === sessionId),
    promptTurns,
    childSessionEdges: db.childSessionEdges.toArray.filter((edge) => promptTurnIds.has(edge.parentPromptTurnId)),
    chunks: db.chunks.toArray.filter((chunk) => promptTurnIds.has(chunk.promptTurnId)),
  }
}

function peerSessionRows(db, promptText, chunkText) {
  const promptTurn = db.promptTurns.toArray.find((turn) => String(turn.text ?? '').includes(promptText))
    ?? db.promptTurns.toArray.find((turn) => turn.parentPromptTurnId)
    ?? null
  const sessionId = promptTurn?.sessionId ?? null
  if (!sessionId) {
    return {
      sessionId: null,
      sessions: [],
      promptTurns: [],
      childSessionEdges: [],
      chunks: db.chunks.toArray.filter((chunk) => String(chunk.content ?? '').includes(chunkText)),
    }
  }
  return latestSessionRows(db, sessionId)
}

async function startHarness(name, serverUrl, stateStream) {
  return compose(sandbox(), middleware([peer()]), agent([agentBin]))
    .start({ serverUrl, name, stateStream })
}

const handles = []
const dbs = []
const acps = []

try {
  const [agentA, agentB] = await Promise.all([
    startHarness(agentAName, serverA, stateStreamA),
    startHarness(agentBName, serverB, stateStreamB),
  ])
  handles.push(agentA, agentB)

  const dbA = await fireline.db({ stateStreamUrl: agentA.state.url })
  const dbB = await fireline.db({ stateStreamUrl: agentB.state.url })
  dbs.push(dbA, dbB)

  const acpA = await agentA.connect('peer-to-peer-demo')
  acps.push(acpA)

  const { sessionId } = await acpA.newSession({ cwd: repoRoot, mcpServers: [] })

  await acpA.prompt({
    sessionId,
    prompt: [{ type: 'text', text: toolCall('list_peers') }],
  })
  await waitForRows(
    dbA.chunks,
    () => {
      const text = readSessionText(dbA, sessionId)
      return text.includes(agentAName) && text.includes(agentBName)
    },
    10_000,
  )
  const listPeersText = readSessionText(dbA, sessionId)

  await acpA.prompt({
    sessionId,
    prompt: [{
      type: 'text',
      text: toolCall('prompt_peer', {
        agentName: agentBName,
        prompt: JSON.stringify({ command: 'echo', message: promptMessage }),
      }),
    }],
  })

  await waitForRows(
    dbA.chunks,
    () => readSessionText(dbA, sessionId).includes(promptMessage),
    10_000,
  )
  await waitForRows(
    dbB.chunks,
    (rows) => rows.some((row) => String(row.content ?? '').includes(promptMessage)),
    10_000,
  )

  const promptPeerText = readSessionText(dbA, sessionId)
  const summary = {
    topology: {
      streamsBaseUrl: `http://127.0.0.1:${process.env.STREAMS_PORT}/v1/stream`,
      serverA,
      serverB,
      sharedDiscoveryRequired: true,
    },
    handles: {
      agentA: {
        name: agentAName,
        acpUrl: agentA.acp.url,
        stateStreamUrl: agentA.state.url,
      },
      agentB: {
        name: agentBName,
        acpUrl: agentB.acp.url,
        stateStreamUrl: agentB.state.url,
      },
    },
    interaction: {
      sessionId,
      listPeersText,
      promptPeerText,
      promptMessage,
    },
    state: {
      agentA: latestSessionRows(dbA, sessionId),
      agentB: peerSessionRows(
        dbB,
        JSON.stringify({ command: 'echo', message: promptMessage }),
        promptMessage,
      ),
    },
    limitations: {
      isolatedFirelineRunStreamsFailDiscovery: true,
      traceparentForwardedAcrossPeerHop: false,
      rawBsideAcpTapCapturedByThisScript: false,
    },
  }

  const summaryPath = path.join(runDir, 'summary.json')
  await fs.mkdir(runDir, { recursive: true })
  await fs.writeFile(summaryPath, JSON.stringify(summary, null, 2))

  console.log(JSON.stringify({
    summaryPath,
    listPeersExcerpt: listPeersText,
    promptPeerExcerpt: promptPeerText,
    agentAStateStream: agentA.state.url,
    agentBStateStream: agentB.state.url,
    note: 'traceparent forwarding is a known failure and is not captured by this script',
  }, null, 2))
} finally {
  await Promise.allSettled(acps.map((acp) => acp.close()))
  await Promise.allSettled(handles.map((handle) => handle.destroy()))
  await Promise.allSettled(dbs.map((db) => db.close()))
}
EOF
}

teardown() {
  setup_dirs
  for label in control-plane-b control-plane-a fireline-streams; do
    local pid_file="$PID_DIR/$label.pid"
    if ! pid_is_live "$pid_file"; then
      rm -f "$pid_file"
      continue
    fi
    local pid
    pid=$(cat "$pid_file")
    log "stopping $label (pid $pid)"
    kill "$pid" >/dev/null 2>&1 || true
    for _ in $(seq 1 50); do
      if ! kill -0 "$pid" >/dev/null 2>&1; then
        break
      fi
      sleep 0.1
    done
    if kill -0 "$pid" >/dev/null 2>&1; then
      kill -9 "$pid" >/dev/null 2>&1 || true
    fi
    wait "$pid" 2>/dev/null || true
    rm -f "$pid_file"
  done
}

case "$MODE" in
  full)
    rm -rf "$RUN_DIR"
    trap teardown EXIT
    setup
    driver
    teardown
    trap - EXIT
    ;;
  setup-only)
    rm -rf "$RUN_DIR"
    setup
    ;;
  driver-only)
    driver
    ;;
  teardown-only)
    teardown
    ;;
  *)
    printf 'usage: %s [full|setup-only|driver-only|teardown-only]\n' "${BASH_SOURCE[0]}" >&2
    exit 1
    ;;
esac
