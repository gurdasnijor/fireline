# Fleet UI Positioning

> Status: decision doc
> Date: 2026-04-12
> Scope: packaging direction for the existing observation UI and trace-backend choice for fleet lineage

This document makes two product decisions for Opus 1:

1. how Fireline should package the observation surface that already exists in `examples/flamecast-client/`
2. how that surface should render cross-agent lineage once canonical W3C trace propagation lands

This is not a full execution plan. `examples/flamecast-client/` is already a working UI. The question is how to position it as a product and what minimal wiring is required to make the demo story honest.

## Current State

`examples/flamecast-client/` is already much more than a toy page:

- it has a branded app shell with sidebar navigation, breadcrumbing, and session/runtime-oriented routes
- it already exposes routes for home, runtimes, sessions, queue, agents, and settings
- it already provisions and lists runtimes through `SandboxAdmin`
- it already opens `fireline.db()` against a shared state stream and uses live queries for sessions, prompt activity, and permissions
- it already connects to ACP directly for active session control through `use-acp`
- it already renders live transcripts, read-only prior sessions, pending approvals, runtime filesystem views, terminal views, queue state, and agent-template CRUD
- `server.ts` already demonstrates the backend façade needed to stitch together Fireline runtime/session operations into an operator-facing UI

At the same time, Flamecast is still example-coupled:

- it is branded and named as `Flamecast`, not as a Fireline product surface
- its backend is a monolithic example server with in-memory template, settings, session, and queue state
- its route and type vocabulary still reflects example concerns rather than a published product boundary
- its current "traces" views are session-local logs, not a true OpenTelemetry lineage graph
- it does not yet integrate with an OTel backend for cross-agent causality

That means the repo already has a viable UI seed, but not yet a stable first-party product boundary.

## Decision 1: Product Positioning

### Options

| Option | Upside | Downside | Fit for Opus 1 |
|---|---|---|---|
| `(a)` Promote `flamecast-client` directly into the first-party product | Fastest path to a polished-looking demo; minimum packaging work; preserves existing UI momentum | Freezes example naming, example server assumptions, and example API seams into the product; higher maintenance burden immediately | Strong for demo speed, weak for long-term product hygiene |
| `(b)` Keep `flamecast-client` as the reference implementation and ship a separate minimal first-party fleet UI product | Preserves a clean product boundary while still reusing most of the existing UI; allows product naming and scope to be Fireline-native; limits long-term support surface | Slightly more packaging work than `(a)`; requires discipline to avoid a hidden second codebase | Best balance of demo speed and maintenance |
| `(c)` Ship nothing first-party and document the pattern only | Lowest product maintenance; no packaging commitment | Undercuts the OpenClaw-style demo narrative; weakens the "polished operational surface" story; makes the observation story feel hypothetical | Poor fit for Opus 1 demo goals |

### Recommendation

Choose **`(b)`**: keep `examples/flamecast-client/` as the reference implementation and ship a separate minimal first-party fleet UI product, tentatively `Fireline Dashboard` (`@fireline/dashboard` or equivalent app packaging).

Why this is the right cut:

- the demo in [`docs/demos/pi-acp-to-openclaw.md`](../demos/pi-acp-to-openclaw.md) explicitly wants product feel, not just architecture diagrams
- shipping nothing first-party would undercut the strongest part of the Fireline story: "operators can observe many agents from one place"
- promoting Flamecast wholesale would optimize for the next demo week but would immediately turn example-specific choices into product promises
- the current UI is already polished enough that the product can start as a narrow repackage and trim, not as a new design effort

Pragmatic interpretation:

- for the demo, it is acceptable if the first `Fireline Dashboard` build is mostly Flamecast under a new product name
- the decision here is about packaging and support boundary, not about rewriting the UI
- Flamecast remains valuable as a living example/reference implementation even after the product shell exists

In short: **reuse the Flamecast code aggressively, but do not make Flamecast-the-example the long-term product contract.**

## Decision 2: OTel Trace Visualization Integration

### Context

The lineage graph promised by the pi-acp -> OpenClaw story cannot be sourced from Fireline-specific edge tables or synthetic row state.

Per [`acp-canonical-identifiers.md`](./acp-canonical-identifiers.md), cross-session causality lives in ACP `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage`, and the trace tree is the lineage. Per [`acp-canonical-identifiers-execution.md` Phase 4](./acp-canonical-identifiers-execution.md#phase-4-w3c-trace-context-propagation), Fireline will propagate that W3C trace context across peer calls and emit OpenTelemetry spans.

The fleet UI therefore needs a real trace backend.

### Options

| Option | Upside | Downside | Fit for Opus 1 |
|---|---|---|---|
| Jaeger embedded | Fastest path to a usable trace graph; familiar UI for spans and parent/child relationships; easy to demo; lower wiring cost than a full observability stack | Another UI surface to embed or deep-link; not necessarily the final long-term backend choice | Best fit for demo pressure and first useful lineage view |
| Tempo + Grafana | Stronger long-term ops story; good fit for larger observability stacks | Heavier setup, more moving parts, weaker "single polished Fireline surface" feel for the first demo | Better later than now |
| DIY lightweight viewer from durable-stream trace events | Tight visual control and one branded UI | Wrong source of truth; duplicates OTel backend functionality; high engineering risk; invites lineage drift against canonical-id rules | Reject |

### Recommendation

Choose **Jaeger embedded** for Opus 1 and the first product-facing fleet UI.

Why:

- it is the shortest path from "W3C trace context lands" to "we can actually show a cross-agent lineage graph"
- it matches the canonical-id design: the trace backend, not `fireline.db()`, remains the source of truth for causality
- it is much lighter than adopting Tempo + Grafana as part of the same milestone
- it avoids spending product time on a custom trace viewer before the trace semantics are fully exercised in the backend

Guardrail:

- the Fleet UI should integrate through a thin trace service boundary keyed by OTel trace ids and span ids
- the UI must not invent a Fireline-only lineage store or derive graphs from ad hoc durable-stream events
- if Fireline later wants Tempo or another backend, that should be a backend swap behind the same trace-query seam, not a rewrite of the UI's conceptual model

For Opus 1, the right answer is: **Fireline owns the operator shell; Jaeger owns the span graph.**

## Wiring Work Implied

This is the small amount of work implied by the decisions above, not a full rollout plan.

1. Repackage the UI surface.
   - Create a first-party product shell from the existing Flamecast app and UI library.
   - Keep `examples/flamecast-client/` as the reference implementation and demo sandbox.
   - Narrow the first supported product surface to sessions, runtimes, approvals, queue, and trace entry points.

2. Add OTel backend hookup.
   - Give the product a trace-backend URL/config path.
   - Wire Fireline demo/dev deployments to export spans to Jaeger.
   - Keep trace lookup keyed by canonical trace context, not Fireline-specific ids.

3. Add a trace-view entry point.
   - Add a trace panel, trace tab, or embedded/deep-linked Jaeger view from session/runtime/fleet pages.
   - Scope the first cut to "show me the cross-agent lineage for this session or fleet event."
   - Do not build a bespoke graph engine inside Fireline Dashboard for the first cut.

4. Update vocabulary to canonical terms.
   - UI copy and product docs should say "trace graph", "request", "session", and "tool call", not synthetic lineage terms.
   - The packaged UI should align with canonical-id language as that rollout lands.
   - Observation state from `fireline.db()` stays agent-plane; infrastructure and trace data come from their own surfaces.

## Dependencies

Hard blocker:

- [`acp-canonical-identifiers-execution.md` Phase 4](./acp-canonical-identifiers-execution.md#phase-4-w3c-trace-context-propagation) must land before the Fleet UI can honestly claim a real cross-agent lineage graph

Why that blocker matters:

- until `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage` propagate end to end, any lineage view is either partial or built on the wrong substrate
- the dashboard can ship earlier as a sessions/runtimes/approvals surface, but the OpenClaw-style "fleet graph" claim must wait for real trace propagation

Related but non-blocking cleanup:

- reducing example-specific naming in the packaged shell
- trimming the product surface down from the full Flamecast demo surface
- aligning copy and data access with canonical-id vocabulary as the state/read-model cleanup lands

## Architect Review Checklist

- [ ] Decision 1 is explicit: Flamecast remains the reference implementation; the supported product surface is a separate first-party dashboard package/app.
- [ ] The recommendation still meets demo pressure: the first dashboard build may be a thin repackage of Flamecast, not a new UI architecture.
- [ ] Decision 2 is explicit: Jaeger embedded is the first OTel integration target.
- [ ] The fleet lineage graph is sourced from OTel spans, not from `fireline.db()` or any Fireline-specific edge table.
- [ ] `fireline.db()` remains an agent-plane observation surface and is not stretched into a trace backend.
- [ ] Canonical W3C trace propagation is called out as a hard dependency before claiming real cross-agent lineage.
- [ ] The doc does not commit Fireline to a full Grafana/Tempo stack for Opus 1.
- [ ] The product vocabulary will align with canonical-id language rather than preserving Flamecast/example-specific terms as public product terms.

## References

- [`examples/flamecast-client/`](../../examples/flamecast-client/)
- [`docs/demos/pi-acp-to-openclaw.md`](../demos/pi-acp-to-openclaw.md)
- [`docs/proposals/proposal-index.md`](./proposal-index.md)
- [`docs/proposals/acp-canonical-identifiers.md`](./acp-canonical-identifiers.md)
- [`docs/proposals/acp-canonical-identifiers-execution.md`](./acp-canonical-identifiers-execution.md)
