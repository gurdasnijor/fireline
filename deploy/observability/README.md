# Observability deploy — Betterstack wiring

This directory holds the operator-side config for pointing Fireline's OTel
export at a Betterstack source. The Fireline host consumes standard
`OTEL_EXPORTER_OTLP_*` environment variables, so wiring Betterstack is a
pure env-var exercise — no code changes required once OTel Phase 2 spans
(`docs/proposals/observability-integration.md §Phase 2`) are emitting.

## What lives here

- `betterstack.env.example` — template env file. Copy to `betterstack.env`
  locally, fill in the source token, source into the operator shell.
- (future) `dashboard/` — saved-view exports from the Betterstack UI once
  the T5 dashboard is stood up.

## Setup

1. Obtain the source token from the Betterstack source settings page
   (`Telemetry → Sources → fireline → Connect OpenTelemetry`).
2. Copy the env template:
   ```bash
   cp deploy/observability/betterstack.env.example deploy/observability/betterstack.env
   ```
3. Edit `betterstack.env` and replace `<SOURCE_TOKEN>` with the real token.
   The `.env` file is gitignored and must never be committed.
4. Source the env file in the operator shell before launching the host:
   ```bash
   set -a; source deploy/observability/betterstack.env; set +a
   npx fireline demo/agent.ts
   ```
5. Smoke-test ingestion (does not require Fireline spans yet):
   ```bash
   curl -X POST \
     -H 'Content-Type: application/json' \
     -H "Authorization: $OTEL_EXPORTER_OTLP_HEADERS" \
     -d "{\"dt\":\"$(date -u +'%Y-%m-%d %T UTC')\",\"message\":\"fireline smoke\"}" \
     $OTEL_EXPORTER_OTLP_ENDPOINT
   ```
   Expect HTTP 202 Accepted.

## Ingestion contract

- Platform: OpenTelemetry
- Protocol: OTLP/HTTP JSON
- Region: us-east-9
- Host: `s2356898.us-east-9.betterstackdata.com`
- Auth: `Authorization: Bearer <SOURCE_TOKEN>`

## Dashboard view (T5 stage)

Target: render the trace tree for the demo's pi-acp → OpenClaw flow in a
single saved view. Required panels:

1. **Session timeline** — time-bucketed count of `fireline.session.created`
   spans, split by `fireline.session_id`.
2. **Prompt request latency** — p50/p95/p99 of `fireline.prompt.request`
   span duration, split by request outcome (ok/error).
3. **Tool call activity** — heatmap of `fireline.tool.call` span starts
   over the demo window, split by `fireline.tool_name`.
4. **Approval timeline** — paired view of `fireline.approval.requested`
   and `fireline.approval.resolved` spans; row-level detail showing
   `fireline.allow` and `fireline.resolved_by` attrs.
5. **Trace tree for demo session** — Betterstack's default trace view
   filtered to the current `fireline.session_id` so the audience sees
   parent/child lineage across session → prompt → tool → approval.

Saved-view URL will land here once the T5 worker stands it up.

## Do-not-commit list

- `betterstack.env` (token-bearing)
- Any raw span exports containing PII
- Dashboard exports that embed the source token in their header config
