# Observability Integration

> Status: spec
> Date: 2026-04-12
> Scope: Fireline OpenTelemetry span emission, W3C trace-context propagation, OTLP export, and verification gates for cross-agent observability

## TL;DR

Fireline does not ship a fleet UI product.

Fireline ships the instrumentation that makes any OTLP-compatible OpenTelemetry backend render:

- session and prompt lineage
- tool-call timing
- approval request and resolution timelines
- cross-agent causality
- subscriber and webhook handling paths

The wire contract for lineage is ACP `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage`, not a Fireline-specific trace id or a bespoke lineage table. The trace tree is the lineage graph.

Text diagram:

```text
Fireline host spans
  -> OTLP export
  -> collector / backend of user's choice
  -> trace tree in their UI
```

This proposal replaces the retracted fleet-UI positioning work. `examples/flamecast-client/` remains an example proving code-footprint reduction, not a product Fireline ships.

## Acceptance Criteria

This document is governed by the same bar as [`acp-canonical-identifiers.md`](./acp-canonical-identifiers.md):

1. No synthetic Fireline lineage table exists for cross-session or cross-agent work. Cross-session causality is handled by OpenTelemetry spans plus ACP `_meta` trace propagation.
2. No new Fireline-invented semantic identifiers appear on spans. Fireline uses canonical ACP identifiers in attributes:
   - `fireline.session_id`
   - `fireline.request_id`
   - `fireline.tool_call_id`
3. Standard OTel span ids and parent relationships are the only span-graph identity surface.
4. `_meta.traceparent` is the wire-format contract for distributed trace continuity. Fireline must not introduce `fireline.traceId`, `_meta.fireline.traceId`, `parentPromptTurnId`, or any equivalent substitute.
5. Agent-layer state remains the source of truth for durable state. Spans are observation, not state.
6. Fireline exports via standard OTLP configuration and does not bundle or require any specific backend.

Dependencies:

- Blocked by [`acp-canonical-identifiers-execution.md` Phase 4](./acp-canonical-identifiers-execution.md#phase-4-w3c-trace-context-propagation)
- Parallel with canonical-identifiers Phases 5-8
- Unblocks Step 5 of [`pi-acp-to-openclaw.md`](../demos/pi-acp-to-openclaw.md): the shared message board and lineage graph render in the user's OTel backend, not in a bespoke Fireline UI

## Span Catalog

The initial span set must use these exact names.

| Span name | Layer | Parent relationship | Required Fireline attrs | Other attrs / conventions |
|---|---|---|---|---|
| `fireline.session.created` | agent session lifecycle | root span or child of incoming `traceparent` when session creation is causally linked to an upstream request | `fireline.session_id` | `rpc.system="jsonrpc"` when appropriate |
| `fireline.prompt.request` | agent session lifecycle | child of incoming `traceparent` if present; otherwise child of `fireline.session.created` or local root | `fireline.session_id`, `fireline.request_id` | `rpc.system="jsonrpc"`, request method attrs when available |
| `fireline.tool.call` | agent session lifecycle | child of `fireline.prompt.request` | `fireline.session_id`, `fireline.tool_call_id` | tool name attrs under `fireline.tool_name`; use standard error status on failure |
| `fireline.approval.requested` | approval workflow | child of the blocked `fireline.prompt.request` | `fireline.session_id`, `fireline.request_id`, `fireline.policy_id`, `fireline.reason` | approval mode attrs under `fireline.approval_mode` if needed |
| `fireline.approval.resolved` | approval workflow | child of `fireline.approval.requested` or same parent chain if externally resolved later | `fireline.session_id`, `fireline.request_id`, `fireline.allow`, `fireline.resolved_by` | preserve original trace context even when the resolver is external |
| `fireline.peer.call.out` | cross-agent call path | child of caller `fireline.prompt.request` | caller `fireline.session_id`, caller `fireline.request_id` | inject `_meta.traceparent`; set `rpc.system="jsonrpc"` and ACP method attrs where available |
| `fireline.peer.call.in` | cross-agent call path | child span of extracted inbound `traceparent` | callee `fireline.session_id` | joins the same trace started by `fireline.peer.call.out` |
| `fireline.sandbox.provisioned` | infrastructure | child of the provisioning flow that created or resumed the runtime | `fireline.session_id` when tied to a session | `fireline.provider`, `fireline.host_key`; infrastructure attrs are allowed here because this is an infrastructure span |
| `fireline.subscriber.handle` | subscriber driver | child of the matched source event's trace | `fireline.subscriber_name`, `fireline.completion_key_variant`, `fireline.handler_outcome` | may also carry `fireline.session_id`, `fireline.request_id`, or `fireline.tool_call_id` when the completion key exposes them |
| `fireline.webhook.delivery` | outbound subscriber side effect | child of `fireline.subscriber.handle` | `fireline.url`, `fireline.retry_attempt` | use `http.method`, `http.url` / `url.full`, and `http.response.status_code` for delivery status |

Notes:

- `fireline.peer.call.out` and `fireline.peer.call.in` are the trace-visible replacement for any synthetic `child_session_edge` lineage model.
- `fireline.sandbox.provisioned` may include infrastructure ids on the span because spans are not agent-layer rows. Do not copy that exception back into `fireline.db()` row design.
- `fireline.subscriber.handle` covers passive and active subscribers. Passive subscribers should still emit the handling span even when the actual completion arrives later from another writer.

## Attribute Conventions

Use these conventions consistently:

- Fireline-specific attributes live under the `fireline.*` namespace.
- Canonical ACP identifiers always use:
  - `fireline.session_id`
  - `fireline.request_id`
  - `fireline.tool_call_id`
- Use standard OTel semantic conventions where applicable:
  - `http.*` and `url.*` for webhook and other HTTP side effects
  - `rpc.*` for ACP / JSON-RPC request handling
  - standard span status for errors
- Do not place infrastructure ids such as `host_key` or `node_id` on agent-layer spans like `fireline.prompt.request`, `fireline.tool.call`, `fireline.approval.requested`, or `fireline.peer.call.out`.
- Infrastructure attrs are acceptable on infrastructure spans such as `fireline.sandbox.provisioned`.

Allowed Fireline-specific attrs in the first cut:

- `fireline.session_id`
- `fireline.request_id`
- `fireline.tool_call_id`
- `fireline.policy_id`
- `fireline.reason`
- `fireline.allow`
- `fireline.resolved_by`
- `fireline.provider`
- `fireline.host_key`
- `fireline.subscriber_name`
- `fireline.completion_key_variant`
- `fireline.handler_outcome`
- `fireline.retry_attempt`
- `fireline.url`

Rejected:

- `fireline.trace_id`
- `fireline.parent_prompt_turn_id`
- `fireline.child_session_id` as a lineage surrogate
- any attribute whose only job is to reconstruct a graph that OTel already provides

## `_meta.traceparent` Propagation Rules

The propagation contract comes from ACP `_meta`, as documented in the ACP extensibility docs and the ACP meta-propagation RFD:

- `_meta` is the extensibility surface
- root-level keys reserved for W3C trace context are:
  - `traceparent`
  - `tracestate`
  - `baggage`

Required propagation:

1. Outbound peer calls:
   - inject `traceparent`, `tracestate`, and `baggage` into the outbound ACP envelope `_meta`
   - `fireline.peer.call.out` opens before dispatch
2. Inbound peer calls:
   - extract `traceparent`, `tracestate`, and `baggage` from inbound ACP `_meta`
   - `fireline.peer.call.in` is a child span of the extracted context
3. Outbound webhook HTTP:
   - inject W3C `traceparent` header
   - propagate `tracestate` and `baggage` headers when present
4. All DurableSubscriber outbound side effects:
   - preserve the source event's trace context
   - apply the same rule whether the side effect is HTTP, ACP, or another process boundary
5. Stream append envelopes:
   - may carry `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage` for observability continuity
   - this is allowed for continuity, but the trace backend, not the stream row model, remains the lineage source of truth

Forbidden propagation shapes:

- `_meta.fireline.traceId`
- `_meta.fireline.parentPromptTurnId`
- `fireline/trace-id`
- any Fireline-namespaced replacement for standard W3C trace-context keys

## OTLP Export Config

Fireline exports traces via standard OpenTelemetry environment variables:

- `OTEL_EXPORTER_OTLP_ENDPOINT`
- `OTEL_EXPORTER_OTLP_HEADERS`
- `OTEL_SERVICE_NAME`
- `OTEL_RESOURCE_ATTRIBUTES`

Rules:

- default `OTEL_SERVICE_NAME` is `fireline`
- if no OTLP endpoint is configured, Fireline should behave as a null-exporter / no-op export path rather than failing normal runtime behavior
- Fireline does not bundle or require a specific backend
- Fireline should export valid OTLP traces to any OTLP-compatible collector chosen by the user

This proposal is traces-only for MVP. Metrics and logs are out of scope.

## Rust Implementation Approach

Use the Rust side as the authoritative emission path.

Workspace additions:

- add `opentelemetry` to the root `Cargo.toml`
- add `opentelemetry-otlp` to the root `Cargo.toml`
- add `tracing-opentelemetry` to the root `Cargo.toml`

Bridge strategy:

- keep `tracing` as the application-facing instrumentation API
- attach a per-crate `tracing-opentelemetry` bridge in crates that already use `tracing`
- resource and exporter configuration live at host bootstrap so emission policy remains centralized

Initial instrumentation entry points:

- session creation and prompt handling
- tool call handling
- approval gate emit / wait / resolve
- peer routing outbound / inbound
- subscriber driver handle path
- webhook delivery path
- sandbox provisioning / resume

Implementation notes:

- spans should be opened in the host/runtime path that already has canonical ACP ids available
- approval spans must use canonical `RequestId`, not the old hashed approval id path
- subscriber spans must expose the completion-key variant in attributes, not a serialized bespoke key string
- infrastructure details such as `host_key` belong on infrastructure spans only

## TypeScript Implementation Approach

`@fireline/client` is not the authoritative span emitter.

Rules:

- the host/runtime emits the authoritative spans
- the TypeScript SDK and client surfaces must preserve `_meta` trace-context fields unchanged
- `@fireline/client` must not strip or rename:
  - `_meta.traceparent`
  - `_meta.tracestate`
  - `_meta.baggage`
- Fireline should add verification that the ACP TypeScript SDK passes `_meta` through unchanged on outbound and inbound envelopes

Non-goal for the TS side:

- the client package does not need its own span emission system for MVP
- it only needs to preserve the wire contract so the host-side trace tree stays coherent

## Phase Plan

### Phase 1: OTel deps + service/resource config + null exporter

Scope:

- add OTel crate dependencies at the workspace root
- add service/resource configuration
- wire a null-exporter or no-op export path when `OTEL_EXPORTER_OTLP_ENDPOINT` is absent
- no behavior change without OTLP config

Verification gate:

- Rust workspace build is green with the new deps
- a targeted test proves Fireline can boot without OTLP config and without span-export errors
- environment precedence is verified: configured `OTEL_*` vars are read, not ignored

### Phase 2: session / prompt / tool / approval spans + peer-out `_meta` injection

Scope:

- instrument:
  - `fireline.session.created`
  - `fireline.prompt.request`
  - `fireline.tool.call`
  - `fireline.approval.requested`
  - `fireline.approval.resolved`
  - `fireline.peer.call.out`
  - `fireline.peer.call.in`
- inject `_meta.traceparent` on outbound peer calls
- extract `_meta.traceparent` on inbound peer calls

Verification gate:

- peer integration test proves outbound `_meta.traceparent` is present
- inbound peer call joins the same trace as the caller span
- source grep for `_meta.fireline.traceId`, `_meta.fireline.parentPromptTurnId`, or equivalent Fireline-specific trace fields returns zero in touched paths

### Phase 3: subscriber / webhook instrumentation + outbound side-effect propagation

Scope:

- instrument:
  - `fireline.subscriber.handle`
  - `fireline.webhook.delivery`
- add required attrs for subscriber name, completion-key variant, handler outcome, URL, status, and retry attempt
- propagate trace context on all DurableSubscriber outbound side effects

Verification gate:

- subscriber/webhook tests prove outbound HTTP carries `traceparent`
- emitted completion envelopes preserve trace continuity
- retry attempts appear on `fireline.webhook.delivery` spans without inventing new semantic ids

### Phase 4: OTLP export smoke test

Scope:

- confirm spans emit in valid OTel protobuf format to any OTLP-compatible collector
- validation may use a local `otlptrace-sniff`-style binary or equivalent collector harness
- no backend-specific setup is part of Fireline's spec

Verification gate:

- smoke test captures exported spans from a running Fireline flow
- exported spans decode as valid OTLP trace payloads
- captured trace tree is coherent for at least:
  - session creation
  - prompt request
  - approval request / resolution or peer call

## Verification Gates by Phase

Phase 1:

- OTel deps compile
- no-op exporter path boots cleanly

Phase 2:

- span names and required attrs exist
- peer trace propagation is end to end via `_meta.traceparent`

Phase 3:

- subscriber side effects propagate trace context
- webhook delivery spans record retry and HTTP outcome attrs

Phase 4:

- OTLP payloads are valid
- trace tree is reconstructible by a generic OTLP-compatible collector

## Non-goals

- a bespoke Fireline UI for trace visualization
- tight coupling to any specific OTel backend
- metrics or logs emission in the MVP
- replacing durable-streams as the source of truth for state
- new Fireline-specific lineage ids, span-linking ids, or edge tables
- turning `examples/flamecast-client/` into a product commitment

## References

- [`acp-canonical-identifiers.md`](./acp-canonical-identifiers.md)
- [`acp-canonical-identifiers-execution.md`](./acp-canonical-identifiers-execution.md)
- [`durable-subscriber.md`](./durable-subscriber.md)
- [`pi-acp-to-openclaw.md`](../demos/pi-acp-to-openclaw.md)
- ACP extensibility docs: https://agentclientprotocol.com/protocol/extensibility#the-_meta-field
- ACP meta-propagation RFD: https://agentclientprotocol.com/rfds/meta-propagation#implementation-details
- ACP agent telemetry export RFD: https://agentclientprotocol.com/rfds/agent-telemetry-export
