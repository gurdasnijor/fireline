#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RUN_ROOT="${RUN_ROOT:-$REPO_ROOT/.tmp/demo-handoff}"
RUN_DIR="${RUN_DIR:-$RUN_ROOT/latest}"
LOG_DIR="$RUN_DIR/logs"
SPEC_DIR="$RUN_DIR/specs"

CLI_ENTRY="${CLI_ENTRY:-$REPO_ROOT/packages/fireline/bin/fireline.js}"
TSX_API="${TSX_API:-$REPO_ROOT/packages/fireline/node_modules/tsx/dist/esm/api/index.mjs}"
SPEC_PATH="${SPEC_PATH:-$REPO_ROOT/docs/demos/assets/agent.ts}"

FIRELINE_BIN="${FIRELINE_BIN:-$REPO_ROOT/target/debug/fireline}"
FIRELINE_STREAMS_BIN="${FIRELINE_STREAMS_BIN:-$REPO_ROOT/target/debug/fireline-streams}"

LOCAL_PORT="${LOCAL_PORT:-4440}"
STREAMS_PORT="${STREAMS_PORT:-7474}"
DOCKER_PORT="${DOCKER_PORT:-4441}"
STATE_STREAM="${STATE_STREAM:-mono-80f-handoff-$(date +%s)}"
SESSION_ID="${SESSION_ID:-}"

LOCAL_HOST_LOG="$LOG_DIR/local-host.log"
BUILD_LOG="$LOG_DIR/build.log"
BUILT_CONTAINER_LOG="$LOG_DIR/built-container.log"
REAL_SPEC_LOG="$LOG_DIR/real-spec-container.log"
OVERRIDE_CONTAINER_LOG="$LOG_DIR/override-container.log"
BUILT_REPL_LOG="$LOG_DIR/built-repl.log"
OVERRIDE_REPL_LOG="$LOG_DIR/override-repl.log"
SUMMARY_JSON="$RUN_DIR/summary.json"
LOCAL_JSON="$RUN_DIR/local-session.json"
BUILT_JSON="$RUN_DIR/built-image.json"
REAL_SPEC_JSON="$RUN_DIR/real-spec.json"
OVERRIDE_JSON="$RUN_DIR/override-spec.json"
FINAL_JSON="$RUN_DIR/final-check.json"

BUILT_CONTAINER=mono-80f-built
REAL_SPEC_CONTAINER=mono-80f-real-spec
OVERRIDE_CONTAINER=mono-80f-override

local_host_pid=

log() {
  printf '[demo-handoff] %s\n' "$*"
}

fail() {
  log "ERROR: $*"
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

require_file() {
  [[ -f "$1" ]] || fail "missing required file: $1"
}

require_executable() {
  [[ -x "$1" ]] || fail "missing required executable: $1"
}

wait_for_http() {
  local url="$1"
  local label="$2"
  local attempts="${3:-60}"
  for _ in $(seq 1 "$attempts"); do
    if curl -fsS "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  return 1
}

cleanup() {
  if [[ -n "${local_host_pid:-}" ]] && kill -0 "$local_host_pid" >/dev/null 2>&1; then
    kill -INT "$local_host_pid" >/dev/null 2>&1 || true
    wait "$local_host_pid" >/dev/null 2>&1 || true
  fi
  docker rm -f "$BUILT_CONTAINER" "$REAL_SPEC_CONTAINER" "$OVERRIDE_CONTAINER" >/dev/null 2>&1 || true
}

trap cleanup EXIT

mkdir -p "$LOG_DIR" "$SPEC_DIR"

require_cmd curl
require_cmd docker
require_cmd node
require_file "$CLI_ENTRY"
require_file "$TSX_API"
require_file "$SPEC_PATH"
require_executable "$FIRELINE_BIN"
require_executable "$FIRELINE_STREAMS_BIN"
[[ -n "${ANTHROPIC_API_KEY:-}" ]] || fail "ANTHROPIC_API_KEY is required for the local and docker prompt legs"

log "step 1: build hosted docker image from $SPEC_PATH"
mkdir -p "$RUN_DIR/build"
(
  cd "$RUN_DIR/build"
  FIRELINE_BIN="$FIRELINE_BIN" \
  FIRELINE_STREAMS_BIN="$FIRELINE_STREAMS_BIN" \
  node "$CLI_ENTRY" build "$SPEC_PATH" --target docker
) | tee "$BUILD_LOG"

IMAGE_TAG=$(sed -n 's/.*image:[[:space:]]*//p' "$BUILD_LOG" | tail -1 | tr -d '\r')
SCAFFOLD_PATH=$(sed -n 's/.*scaffold:[[:space:]]*//p' "$BUILD_LOG" | tail -1 | tr -d '\r')
[[ -n "$IMAGE_TAG" ]] || fail "could not parse built image tag from $BUILD_LOG"
[[ -n "$SCAFFOLD_PATH" ]] || fail "could not parse scaffold path from $BUILD_LOG"
[[ -f "$SCAFFOLD_PATH" ]] || fail "scaffold file missing: $SCAFFOLD_PATH"

log "step 2: start local fireline host and drive two turns through the public client surface"
FIRELINE_BIN="$FIRELINE_BIN" \
FIRELINE_STREAMS_BIN="$FIRELINE_STREAMS_BIN" \
node "$CLI_ENTRY" run "$SPEC_PATH" \
  --port "$LOCAL_PORT" \
  --streams-port "$STREAMS_PORT" \
  --state-stream "$STATE_STREAM" \
  >"$LOCAL_HOST_LOG" 2>&1 &
local_host_pid=$!

wait_for_http "http://127.0.0.1:${LOCAL_PORT}/healthz" "local fireline" 45 \
  || fail "local fireline host did not become healthy on :$LOCAL_PORT"

ACP_URL=$(sed -n 's/.*ACP:[[:space:]]*//p' "$LOCAL_HOST_LOG" | tail -1 | tr -d '\r')
STATE_URL=$(sed -n 's/.*state:[[:space:]]*//p' "$LOCAL_HOST_LOG" | tail -1 | tr -d '\r')
[[ -n "$ACP_URL" ]] || fail "failed to parse ACP URL from $LOCAL_HOST_LOG"
[[ -n "$STATE_URL" ]] || fail "failed to parse state URL from $LOCAL_HOST_LOG"

REPO_ROOT="$REPO_ROOT" \
TSX_API="$TSX_API" \
ACP_URL="$ACP_URL" \
STATE_URL="$STATE_URL" \
LOCAL_JSON="$LOCAL_JSON" \
node --input-type=module <<'EOF'
import fs from 'node:fs/promises'
import { pathToFileURL } from 'node:url'

const repoRoot = process.env.REPO_ROOT
const tsxApi = process.env.TSX_API
const acpUrl = process.env.ACP_URL
const stateUrl = process.env.STATE_URL
const outPath = process.env.LOCAL_JSON

const { tsImport } = await import(pathToFileURL(tsxApi).href)
const clientMod = await tsImport(pathToFileURL(`${repoRoot}/packages/client/src/index.ts`).href, {
  parentURL: pathToFileURL(`${repoRoot}/`).href,
})

for (const method of ['log', 'warn', 'error']) {
  const original = console[method].bind(console)
  console[method] = (...args) => {
    if (typeof args[0] === 'string' && args[0].startsWith('[StreamDB]')) return
    original(...args)
  }
}

const fireline = clientMod.default
const { appendApprovalResolved, connectAcp } = clientMod

async function waitFor(getValue, timeoutMs = 60000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const value = await getValue()
    if (value !== null && value !== undefined) return value
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  return null
}

function chunkTextForRequest(db, requestId) {
  return db.chunks.toArray
    .filter((row) => row.requestId === requestId)
    .map((row) => row.update?.content?.text ?? '')
    .join('')
}

async function runPrompt(db, acp, sessionId, promptText, expectedText) {
  const promptPromise = acp.prompt({
    sessionId,
    prompt: [{ type: 'text', text: promptText }],
  })

  const permission = await waitFor(
    () =>
      db.permissions.toArray.find(
        (row) => row.sessionId === sessionId && row.state === 'pending',
      ) ?? null,
  )
  if (!permission) {
    throw new Error(`approval did not appear for prompt: ${promptText}`)
  }

  await appendApprovalResolved({
    streamUrl: stateUrl,
    sessionId,
    requestId: permission.requestId,
    allow: true,
    resolvedBy: 'demo-handoff-script',
  })

  await promptPromise

  const promptRow = await waitFor(
    () =>
      db.promptRequests.toArray.find(
        (row) => row.sessionId === sessionId && row.text === promptText && row.state === 'completed',
      ) ?? null,
  )
  if (!promptRow) {
    throw new Error(`prompt did not complete: ${promptText}`)
  }

  const responseText = await waitFor(() => {
    const text = chunkTextForRequest(db, promptRow.requestId)
    return text.includes(expectedText) ? text : null
  })
  if (!responseText) {
    throw new Error(`response for prompt did not include expected text '${expectedText}'`)
  }

  return {
    requestId: promptRow.requestId,
    permissionRequestId: permission.requestId,
    responseText,
  }
}

const db = await fireline.db({ stateStreamUrl: stateUrl })
const acp = await connectAcp(acpUrl, 'demo-handoff-local')

try {
  const { sessionId } = await acp.newSession({ cwd: repoRoot, mcpServers: [] })

  const first = await runPrompt(
    db,
    acp,
    sessionId,
    'We are testing remote handoff. Remember the codeword cobalt-kite for later. Reply exactly: stored cobalt-kite',
    'stored cobalt-kite',
  )
  const second = await runPrompt(
    db,
    acp,
    sessionId,
    'Second turn before handoff. Reply exactly: will-remember',
    'will-remember',
  )

  const result = {
    sessionId,
    acpUrl,
    stateUrl,
    turns: [first, second],
  }
  await fs.writeFile(outPath, `${JSON.stringify(result, null, 2)}\n`)
  console.log(JSON.stringify(result, null, 2))
} finally {
  await acp.close().catch(() => {})
  db.close()
}
EOF

SESSION_ID=$(node -e "const fs=require('fs'); const data=JSON.parse(fs.readFileSync(process.argv[1],'utf8')); process.stdout.write(data.sessionId)" "$LOCAL_JSON")
[[ -n "$SESSION_ID" ]] || fail "failed to capture session id from $LOCAL_JSON"

log "step 3: stop local fireline host"
kill -INT "$local_host_pid" >/dev/null 2>&1 || true
wait "$local_host_pid" >/dev/null 2>&1 || true
local_host_pid=

log "step 4a: run the as-built image exactly once"
docker rm -f "$BUILT_CONTAINER" >/dev/null 2>&1 || true
docker run -d \
  --name "$BUILT_CONTAINER" \
  -p "${DOCKER_PORT}:4440" \
  -e ANTHROPIC_API_KEY \
  -e FIRELINE_DURABLE_STREAMS_URL="http://host.docker.internal:${STREAMS_PORT}/v1/stream" \
  -e FIRELINE_ADVERTISED_STATE_STREAM_URL="http://127.0.0.1:${STREAMS_PORT}/v1/stream/${STATE_STREAM}" \
  "$IMAGE_TAG" >/dev/null

wait_for_http "http://127.0.0.1:${DOCKER_PORT}/healthz" "as-built docker host" 45 \
  || true
docker logs "$BUILT_CONTAINER" >"$BUILT_CONTAINER_LOG" 2>&1 || true

REPO_ROOT="$REPO_ROOT" \
BUILT_CONTAINER_LOG="$BUILT_CONTAINER_LOG" \
BUILT_JSON="$BUILT_JSON" \
IMAGE_TAG="$IMAGE_TAG" \
SCAFFOLD_PATH="$SCAFFOLD_PATH" \
node --input-type=module <<'EOF'
import fs from 'node:fs/promises'

const logPath = process.env.BUILT_CONTAINER_LOG
const outPath = process.env.BUILT_JSON
const imageTag = process.env.IMAGE_TAG
const scaffoldPath = process.env.SCAFFOLD_PATH
const logText = await fs.readFile(logPath, 'utf8').catch(() => '')

const result = {
  imageTag,
  scaffoldPath,
  placeholderBootObserved: logText.includes("embedded spec 'placeholder'"),
  logPath,
}

await fs.writeFile(outPath, `${JSON.stringify(result, null, 2)}\n`)
console.log(JSON.stringify(result, null, 2))
EOF

docker rm -f "$BUILT_CONTAINER" >/dev/null 2>&1 || true

log "step 4b: mount the real spec into the built image to confirm the resource-mount blocker"
REPO_ROOT="$REPO_ROOT" \
TSX_API="$TSX_API" \
SPEC_PATH="$SPEC_PATH" \
STATE_STREAM="$STATE_STREAM" \
REAL_SPEC_DIR="$SPEC_DIR/real-spec" \
node --input-type=module <<'EOF'
import fs from 'node:fs/promises'
import { pathToFileURL } from 'node:url'

const repoRoot = process.env.REPO_ROOT
const tsxApi = process.env.TSX_API
const specPath = process.env.SPEC_PATH
const stateStream = process.env.STATE_STREAM
const outDir = process.env.REAL_SPEC_DIR

const { tsImport } = await import(pathToFileURL(tsxApi).href)
const mod = await tsImport(pathToFileURL(specPath).href, {
  parentURL: pathToFileURL(`${repoRoot}/`).href,
})

let candidate = mod.default ?? mod
while (candidate && typeof candidate === 'object' && 'default' in candidate && typeof candidate.start !== 'function') {
  candidate = candidate.default
}

const spec = JSON.parse(JSON.stringify(candidate))
spec.stateStream = stateStream

await fs.mkdir(outDir, { recursive: true })
await fs.writeFile(`${outDir}/spec.json`, `${JSON.stringify(spec, null, 2)}\n`)
EOF

docker rm -f "$REAL_SPEC_CONTAINER" >/dev/null 2>&1 || true
docker run -d \
  --name "$REAL_SPEC_CONTAINER" \
  -p "${DOCKER_PORT}:4440" \
  -e ANTHROPIC_API_KEY \
  -e FIRELINE_DURABLE_STREAMS_URL="http://host.docker.internal:${STREAMS_PORT}/v1/stream" \
  -e FIRELINE_ADVERTISED_STATE_STREAM_URL="http://127.0.0.1:${STREAMS_PORT}/v1/stream/${STATE_STREAM}" \
  -e FIRELINE_EMBEDDED_SPEC_PATH="/var/lib/fireline/spec-override/spec.json" \
  -v "$SPEC_DIR/real-spec:/var/lib/fireline/spec-override:ro" \
  "$IMAGE_TAG" >/dev/null
sleep 2
docker logs "$REAL_SPEC_CONTAINER" >"$REAL_SPEC_LOG" 2>&1 || true
docker rm -f "$REAL_SPEC_CONTAINER" >/dev/null 2>&1 || true

REAL_SPEC_LOG="$REAL_SPEC_LOG" \
REAL_SPEC_JSON="$REAL_SPEC_JSON" \
node --input-type=module <<'EOF'
import fs from 'node:fs/promises'

const logPath = process.env.REAL_SPEC_LOG
const outPath = process.env.REAL_SPEC_JSON
const logText = await fs.readFile(logPath, 'utf8').catch(() => '')

const result = {
  resourceMountFailureObserved: logText.includes('embedded-spec boot does not support resource mounts'),
  logPath,
}

await fs.writeFile(outPath, `${JSON.stringify(result, null, 2)}\n`)
console.log(JSON.stringify(result, null, 2))
EOF

log "step 4c: run a docker-safe override spec against the same built image and try standalone attach"
REPO_ROOT="$REPO_ROOT" \
TSX_API="$TSX_API" \
SPEC_PATH="$SPEC_PATH" \
STATE_STREAM="$STATE_STREAM" \
OVERRIDE_SPEC_DIR="$SPEC_DIR/override-spec" \
node --input-type=module <<'EOF'
import fs from 'node:fs/promises'
import { pathToFileURL } from 'node:url'

const repoRoot = process.env.REPO_ROOT
const tsxApi = process.env.TSX_API
const specPath = process.env.SPEC_PATH
const stateStream = process.env.STATE_STREAM
const outDir = process.env.OVERRIDE_SPEC_DIR

const { tsImport } = await import(pathToFileURL(tsxApi).href)
const mod = await tsImport(pathToFileURL(specPath).href, {
  parentURL: pathToFileURL(`${repoRoot}/`).href,
})

let candidate = mod.default ?? mod
while (candidate && typeof candidate === 'object' && 'default' in candidate && typeof candidate.start !== 'function') {
  candidate = candidate.default
}

const spec = JSON.parse(JSON.stringify(candidate))
spec.stateStream = stateStream
if (spec.sandbox && typeof spec.sandbox === 'object') {
  delete spec.sandbox.resources
}

await fs.mkdir(outDir, { recursive: true })
await fs.writeFile(`${outDir}/spec.json`, `${JSON.stringify(spec, null, 2)}\n`)
EOF

docker rm -f "$OVERRIDE_CONTAINER" >/dev/null 2>&1 || true
docker run -d \
  --name "$OVERRIDE_CONTAINER" \
  -p "${DOCKER_PORT}:4440" \
  -e ANTHROPIC_API_KEY \
  -e FIRELINE_DURABLE_STREAMS_URL="http://host.docker.internal:${STREAMS_PORT}/v1/stream" \
  -e FIRELINE_ADVERTISED_STATE_STREAM_URL="http://127.0.0.1:${STREAMS_PORT}/v1/stream/${STATE_STREAM}" \
  -e FIRELINE_EMBEDDED_SPEC_PATH="/var/lib/fireline/spec-override/spec.json" \
  -v "$SPEC_DIR/override-spec:/var/lib/fireline/spec-override:ro" \
  "$IMAGE_TAG" >/dev/null

wait_for_http "http://127.0.0.1:${DOCKER_PORT}/healthz" "override docker host" 45 \
  || fail "docker-safe override host did not become healthy on :$DOCKER_PORT"
docker logs "$OVERRIDE_CONTAINER" >"$OVERRIDE_CONTAINER_LOG" 2>&1 || true

set +e
FIRELINE_URL="http://127.0.0.1:${DOCKER_PORT}" \
  node "$CLI_ENTRY" repl "$SESSION_ID" >"$OVERRIDE_REPL_LOG" 2>&1
OVERRIDE_REPL_EXIT=$?
set -e

REPO_ROOT="$REPO_ROOT" \
TSX_API="$TSX_API" \
DOCKER_ACP_URL="ws://127.0.0.1:${DOCKER_PORT}/acp" \
STATE_URL="$STATE_URL" \
SESSION_ID="$SESSION_ID" \
FINAL_JSON="$FINAL_JSON" \
node --input-type=module <<'EOF'
import fs from 'node:fs/promises'
import { pathToFileURL } from 'node:url'

const repoRoot = process.env.REPO_ROOT
const tsxApi = process.env.TSX_API
const acpUrl = process.env.DOCKER_ACP_URL
const stateUrl = process.env.STATE_URL
const sessionId = process.env.SESSION_ID
const outPath = process.env.FINAL_JSON

const { tsImport } = await import(pathToFileURL(tsxApi).href)
const clientMod = await tsImport(pathToFileURL(`${repoRoot}/packages/client/src/index.ts`).href, {
  parentURL: pathToFileURL(`${repoRoot}/`).href,
})

for (const method of ['log', 'warn', 'error']) {
  const original = console[method].bind(console)
  console[method] = (...args) => {
    if (typeof args[0] === 'string' && args[0].startsWith('[StreamDB]')) return
    original(...args)
  }
}

const fireline = clientMod.default
const { appendApprovalResolved, connectAcp } = clientMod

async function waitFor(getValue, timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const value = await getValue()
    if (value !== null && value !== undefined) return value
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  return null
}

const db = await fireline.db({ stateStreamUrl: stateUrl })
let acp = null
const result = {
  acpUrl,
  sessionId,
  loadSession: 'not-run',
  finalPrompt: 'not-run',
  finalResponseText: null,
  error: null,
}

try {
  acp = await connectAcp(acpUrl, 'demo-handoff-docker')
  await acp.loadSession({ cwd: repoRoot, sessionId, mcpServers: [] })
  result.loadSession = 'ok'

  const promptText = 'What codeword was stored before restart? Reply exactly with the codeword only.'
  const promptPromise = acp.prompt({
    sessionId,
    prompt: [{ type: 'text', text: promptText }],
  })

  const permission = await waitFor(
    () =>
      db.permissions.toArray.find(
        (row) => row.sessionId === sessionId && row.state === 'pending',
      ) ?? null,
    5000,
  )
  if (permission) {
    await appendApprovalResolved({
      streamUrl: stateUrl,
      sessionId,
      requestId: permission.requestId,
      allow: true,
      resolvedBy: 'demo-handoff-script',
    })
  }

  await promptPromise

  const promptRow = await waitFor(
    () =>
      db.promptRequests.toArray.find(
        (row) => row.sessionId === sessionId && row.text === promptText && row.state === 'completed',
      ) ?? null,
  )
  const finalText =
    promptRow === null
      ? null
      : db.chunks.toArray
          .filter((row) => row.requestId === promptRow.requestId)
          .map((row) => row.update?.content?.text ?? '')
          .join('')
  if (!finalText || !finalText.includes('cobalt-kite')) {
    throw new Error(`final prompt did not replay prior context; got ${JSON.stringify(finalText)}`)
  }

  result.finalPrompt = 'ok'
  result.finalResponseText = finalText
} catch (error) {
  result.error = error instanceof Error ? error.message : String(error)
  if (result.loadSession === 'ok') {
    result.finalPrompt = 'failed'
  } else {
    result.loadSession = 'failed'
  }
} finally {
  if (acp) await acp.close().catch(() => {})
  db.close()
}

await fs.writeFile(outPath, `${JSON.stringify(result, null, 2)}\n`)
console.log(JSON.stringify(result, null, 2))
EOF

OVERRIDE_CONTAINER_LOG="$OVERRIDE_CONTAINER_LOG" \
OVERRIDE_REPL_LOG="$OVERRIDE_REPL_LOG" \
OVERRIDE_REPL_EXIT="$OVERRIDE_REPL_EXIT" \
OVERRIDE_JSON="$OVERRIDE_JSON" \
node --input-type=module <<'EOF'
import fs from 'node:fs/promises'

const containerLogPath = process.env.OVERRIDE_CONTAINER_LOG
const replLogPath = process.env.OVERRIDE_REPL_LOG
const outPath = process.env.OVERRIDE_JSON
const replExit = Number(process.env.OVERRIDE_REPL_EXIT)

const [containerLog, replLog] = await Promise.all([
  fs.readFile(containerLogPath, 'utf8').catch(() => ''),
  fs.readFile(replLogPath, 'utf8').catch(() => ''),
])

const result = {
  healthOk: true,
  replExitCode: replExit,
  replResourceNotFound: replLog.includes('Resource not found'),
  containerLogPath,
  replLogPath,
  embeddedSpecBooted: containerLog.includes('booting embedded spec'),
}

await fs.writeFile(outPath, `${JSON.stringify(result, null, 2)}\n`)
console.log(JSON.stringify(result, null, 2))
EOF

docker rm -f "$OVERRIDE_CONTAINER" >/dev/null 2>&1 || true

log "writing summary to $SUMMARY_JSON"
REPO_ROOT="$REPO_ROOT" \
BUILD_LOG="$BUILD_LOG" \
LOCAL_JSON="$LOCAL_JSON" \
BUILT_JSON="$BUILT_JSON" \
REAL_SPEC_JSON="$REAL_SPEC_JSON" \
OVERRIDE_JSON="$OVERRIDE_JSON" \
FINAL_JSON="$FINAL_JSON" \
SUMMARY_JSON="$SUMMARY_JSON" \
STATE_STREAM="$STATE_STREAM" \
SESSION_ID="$SESSION_ID" \
node --input-type=module <<'EOF'
import fs from 'node:fs/promises'

const local = JSON.parse(await fs.readFile(process.env.LOCAL_JSON, 'utf8'))
const built = JSON.parse(await fs.readFile(process.env.BUILT_JSON, 'utf8'))
const realSpec = JSON.parse(await fs.readFile(process.env.REAL_SPEC_JSON, 'utf8'))
const override = JSON.parse(await fs.readFile(process.env.OVERRIDE_JSON, 'utf8'))
const finalCheck = JSON.parse(await fs.readFile(process.env.FINAL_JSON, 'utf8'))

const summary = {
  executedAt: new Date().toISOString(),
  repoRoot: process.env.REPO_ROOT,
  stateStream: process.env.STATE_STREAM,
  sessionId: process.env.SESSION_ID,
  verdict:
    built.placeholderBootObserved ||
    realSpec.resourceMountFailureObserved ||
    override.replExitCode !== 0 ||
    finalCheck.loadSession !== 'ok' ||
    finalCheck.finalPrompt !== 'ok'
      ? 'fail'
      : 'pass',
  local,
  asBuiltImage: built,
  realSpecMount: realSpec,
  dockerSafeOverride: override,
  finalPromptCheck: finalCheck,
}

await fs.writeFile(process.env.SUMMARY_JSON, `${JSON.stringify(summary, null, 2)}\n`)
console.log(JSON.stringify({
  summaryPath: process.env.SUMMARY_JSON,
  verdict: summary.verdict,
  sessionId: summary.sessionId,
  stateStream: summary.stateStream,
  placeholderBootObserved: built.placeholderBootObserved,
  resourceMountFailureObserved: realSpec.resourceMountFailureObserved,
  replExitCode: override.replExitCode,
  loadSession: finalCheck.loadSession,
  finalPrompt: finalCheck.finalPrompt,
}, null, 2))
EOF

if node -e "const fs=require('fs'); const summary=JSON.parse(fs.readFileSync(process.argv[1],'utf8')); process.exit(summary.verdict === 'pass' ? 0 : 1)" "$SUMMARY_JSON"; then
  log "Preflight handoff smoke passed"
else
  log "Handoff smoke failed; see $SUMMARY_JSON"
  exit 1
fi
