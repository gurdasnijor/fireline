#!/usr/bin/env bash
# Inject a synthetic OTLP trace into the configured Betterstack source so
# the T5.2 dashboard panels can be built before real spans emit from T4.1.
#
# Usage:
#   set -a; source deploy/observability/betterstack.env; set +a
#   ./deploy/observability/dashboard/inject-synthetic.sh
#
# Requires: curl, jq
# Env:
#   OTEL_EXPORTER_OTLP_ENDPOINT   (from betterstack.env)
#   OTEL_EXPORTER_OTLP_HEADERS    (Authorization=Bearer <SOURCE_TOKEN>)
#
# Emits five spans matching the Phase 2 catalog so each dashboard panel
# has data to render against.

set -euo pipefail

: "${OTEL_EXPORTER_OTLP_ENDPOINT:?source deploy/observability/betterstack.env first}"
: "${OTEL_EXPORTER_OTLP_HEADERS:?source deploy/observability/betterstack.env first}"

AUTH_HEADER="${OTEL_EXPORTER_OTLP_HEADERS/Authorization=/Authorization: }"
ENDPOINT="${OTEL_EXPORTER_OTLP_ENDPOINT%/}"

# Simple line-per-event JSON to the ingest host. Betterstack accepts this for
# basic source ingestion; full OTLP/HTTP protobuf emission is T4.1's job.
# This is ONLY a smoke utility to prove the dashboard can query span names.

ts() { date -u +'%Y-%m-%d %H:%M:%S UTC'; }
SESSION_ID="synthetic-session-$(date +%s)"
REQUEST_ID="synthetic-req-$(date +%s)"
TOOL_CALL_ID="synthetic-tool-$(date +%s)"

post() {
  local name="$1"; shift
  local payload
  payload=$(jq -c --arg dt "$(ts)" --arg span "$name" --arg session "$SESSION_ID" \
    --arg request "$REQUEST_ID" --arg tool "$TOOL_CALL_ID" \
    '{dt: $dt, message: "fireline span \($span)", attrs: {span_name: $span, "fireline.session_id": $session, "fireline.request_id": $request, "fireline.tool_call_id": $tool}} + $ARGS.named' \
    --argjson extra "${1:-{\}}" 2>/dev/null || echo "{}")
  curl -fsSL -X POST \
    -H 'Content-Type: application/json' \
    -H "${AUTH_HEADER}" \
    -d "${payload}" \
    "${ENDPOINT}" > /dev/null
  echo "  injected ${name}"
}

echo "Injecting synthetic demo trace (session=${SESSION_ID})"

post 'fireline.session.created'
sleep 1
post 'fireline.prompt.request'
sleep 1
post 'fireline.tool.call'
sleep 1
post 'fireline.approval.requested'
sleep 2
post 'fireline.approval.resolved'

echo "Done. Query your Betterstack source for span_name:fireline.* to build panels."
