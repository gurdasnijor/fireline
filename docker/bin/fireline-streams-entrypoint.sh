#!/usr/bin/env bash
set -euo pipefail

if (($# > 0)); then
  exec "$@"
fi

: "${FIRELINE_STREAMS_PORT:=7474}"
: "${FIRELINE_STREAMS_INTERNAL_PORT:=17474}"
: "${DS_STORAGE__MODE:=file-durable}"
: "${DS_STORAGE__DATA_DIR:=/var/lib/fireline/durable-streams}"

mkdir -p "$DS_STORAGE__DATA_DIR"

cleanup() {
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

wait -n "$streams_pid" "$proxy_pid"
exit $?
