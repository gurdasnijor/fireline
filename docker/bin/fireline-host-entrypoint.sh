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
: "${FIRELINE_DURABLE_STREAMS_URL:=http://fireline-streams:7474/v1/stream}"

args=(
  /usr/local/bin/fireline
  --control-plane
  --host "$FIRELINE_HOST"
  --port "$FIRELINE_PORT"
  --name "$FIRELINE_NAME"
  --provider "$FIRELINE_CONTROL_PLANE_PROVIDER"
  --durable-streams-url "$FIRELINE_DURABLE_STREAMS_URL"
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

exec "${args[@]}"
