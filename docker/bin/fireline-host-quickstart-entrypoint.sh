#!/usr/bin/env bash
set -euo pipefail

if (($# > 0)); then
  if [[ "$1" == -* ]]; then
    exec /usr/local/bin/fireline "$@"
  fi
  exec "$@"
fi

: "${FIRELINE_PORT:=4440}"
: "${FIRELINE_HOST:=0.0.0.0}"
: "${FIRELINE_NAME:=hosted-fireline}"
: "${FIRELINE_CONTROL_PLANE_PROVIDER:=local}"
: "${FIRELINE_STREAMS_PORT:=7474}"
: "${FIRELINE_STREAMS_INTERNAL_PORT:=17474}"
: "${DS_STORAGE__MODE:=file-durable}"
: "${DS_STORAGE__DATA_DIR:=/var/lib/fireline/durable-streams}"

mkdir -p "$DS_STORAGE__DATA_DIR"

cleanup() {
  if [[ -n "${host_pid:-}" ]]; then
    kill "$host_pid" 2>/dev/null || true
    wait "$host_pid" 2>/dev/null || true
  fi
  if [[ -n "${proxy_pid:-}" ]]; then
    kill "$proxy_pid" 2>/dev/null || true
    wait "$proxy_pid" 2>/dev/null || true
  fi
  if [[ -n "${streams_pid:-}" ]]; then
    kill "$streams_pid" 2>/dev/null || true
    wait "$streams_pid" 2>/dev/null || true
  fi
}

trap cleanup EXIT INT TERM

PORT="$FIRELINE_STREAMS_INTERNAL_PORT" /usr/local/bin/fireline-streams &
streams_pid=$!

socat "TCP-LISTEN:${FIRELINE_STREAMS_PORT},fork,reuseaddr,bind=0.0.0.0" "TCP:127.0.0.1:${FIRELINE_STREAMS_INTERNAL_PORT}" &
proxy_pid=$!

streams_ready=0
for _ in $(seq 1 30); do
  if curl -fsS "http://127.0.0.1:${FIRELINE_STREAMS_INTERNAL_PORT}/healthz" >/dev/null; then
    streams_ready=1
    break
  fi
  sleep 1
done

if [[ "$streams_ready" -ne 1 ]]; then
  echo "fireline-streams did not become healthy on the internal port" >&2
  exit 1
fi

args=(
  /usr/local/bin/fireline
  --control-plane
  --host "$FIRELINE_HOST"
  --port "$FIRELINE_PORT"
  --name "$FIRELINE_NAME"
  --provider "$FIRELINE_CONTROL_PLANE_PROVIDER"
  --durable-streams-url "http://127.0.0.1:${FIRELINE_STREAMS_INTERNAL_PORT}/v1/stream"
)

if [[ -n "${FIRELINE_PEER_DIRECTORY_PATH:-}" ]]; then
  args+=(--peer-directory-path "$FIRELINE_PEER_DIRECTORY_PATH")
fi

if [[ -n "${FIRELINE_STARTUP_TIMEOUT_MS:-}" ]]; then
  args+=(--startup-timeout-ms "$FIRELINE_STARTUP_TIMEOUT_MS")
fi

if [[ -n "${FIRELINE_STOP_TIMEOUT_MS:-}" ]]; then
  args+=(--stop-timeout-ms "$FIRELINE_STOP_TIMEOUT_MS")
fi

if [[ -n "${FIRELINE_DOCKER_BUILD_CONTEXT:-}" ]]; then
  args+=(--docker-build-context "$FIRELINE_DOCKER_BUILD_CONTEXT")
fi

if [[ -n "${FIRELINE_DOCKERFILE:-}" ]]; then
  args+=(--dockerfile "$FIRELINE_DOCKERFILE")
fi

if [[ -n "${FIRELINE_DOCKER_IMAGE:-}" ]]; then
  args+=(--docker-image "$FIRELINE_DOCKER_IMAGE")
fi

if [[ -n "${FIRELINE_ADVERTISED_ACP_URL:-}" ]]; then
  args+=(--advertised-acp-url "$FIRELINE_ADVERTISED_ACP_URL")
fi

if [[ -n "${FIRELINE_ADVERTISED_STATE_STREAM_URL:-}" ]]; then
  args+=(--advertised-state-stream-url "$FIRELINE_ADVERTISED_STATE_STREAM_URL")
fi

"${args[@]}" &
host_pid=$!

wait -n "$streams_pid" "$proxy_pid" "$host_pid"
exit $?
