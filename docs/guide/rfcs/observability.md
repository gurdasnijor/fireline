# RFC: Observability

> Status: design rationale
> Audience: engineers deciding whether Fireline observability should come from durable state plus standard tracing or from a bespoke Fireline graph layer

Fireline does not need its own lineage UI to be observable.

It needs one honest observation story:

- durable streams are the source of truth for state
- OpenTelemetry spans are the source of truth for execution lineage
- ACP `_meta.traceparent`, `_meta.tracestate`, and `baggage` are the wire contract that keeps that lineage intact across hops

That is the design.

## The Problem Fireline Is Avoiding

Observability systems drift when they try to do two jobs with one mechanism.

One layer wants replayable product state. Another wants a causal graph across agents, hosts, and external systems. A third wants a dashboard that "just shows everything." The common mistake is to solve all three by inventing a product-specific lineage table or trace id.

That feels efficient until it starts to collapse:

- state rows begin carrying observability-only fields
- traces stop matching the durable workflow model
- cross-process hops need Fireline-specific glue instead of standard propagation
- the product becomes coupled to one internal visualization surface

Fireline is choosing not to build that trap.

## The Decision

Fireline splits observation into two complementary truths:

- durable state answers what happened in the workflow
- traces answer how execution moved across process and host boundaries

Those truths meet through canonical identifiers and W3C trace context, not through a bespoke Fireline lineage graph.

In practice, that means:

- session, request, and tool-call identity stay canonical
- spans use those canonical ids as attributes
- outbound peer calls, subscriber side effects, and webhook deliveries propagate trace context
- the user chooses any OTLP-compatible backend to render the trace tree

Fireline supplies the contract. It does not need to own the backend UI.

## Why State And Traces Need Different Jobs

Durable state and tracing are both observation surfaces, but they answer different questions.

Durable state is where you look for facts that must survive replay:

- was this approval requested
- was it resolved
- what chunks were emitted
- what durable completion exists for this wait

Tracing is where you look for causality and timing:

- which prompt led to this webhook
- which peer call continued this trace
- how long the subscriber handler spent before retry
- where a workflow paused before an awakeable resolved

If spans tried to become durable state, they would be the wrong storage model.

If state rows tried to become the lineage graph, they would start carrying synthetic edges and observability-only baggage.

Fireline stays clean by letting each surface do the job it is good at.

## Why Canonical IDs Matter Here Too

Observability only becomes coherent if the trace story and the state story speak the same identity language.

That is why this RFC depends on the canonical-identifiers decision:

- `fireline.session_id` should mean ACP `SessionId`
- `fireline.request_id` should mean ACP `RequestId`
- `fireline.tool_call_id` should mean ACP `ToolCallId`

The trace tree is then talking about the same logical work items the durable stream is recording.

That matters for every user-facing flow:

- approvals
- durable subscribers
- awakeables
- peer routing
- webhook callbacks

Without that alignment, the backend would show one set of ids while the durable state model exposed another. Fireline would be observable, but not intelligible.

## One Trace Story Across Boundaries

The observability design has a strong bias: every cross-boundary hop should stay on one trace unless there is a real reason to fork it.

That is why Fireline treats W3C trace propagation as a first-class contract:

- outbound peer calls carry `_meta.traceparent`, `_meta.tracestate`, and `baggage`
- inbound peer calls join the existing trace instead of inventing a sibling lineage record
- subscriber side effects preserve the same trace context as the source event
- webhook deliveries can appear inside the same distributed trace as the prompt that triggered them
- awakeable resolution and approval resolution can still be tied back to the original durable wait

This is the user-facing payoff: an engineer can open a trace in their backend and follow the real causal path instead of reconstructing it from product-specific clues.

## Why Fireline Does Not Ship A Fleet Graph Product

The tempting alternative is to build a Fireline-native graph view and treat that as the primary observability surface.

Fireline is explicitly choosing not to.

The problem is not that a graph UI is useless. The problem is that once the product owns a bespoke graph model, the model itself becomes another architecture seam to preserve. Then traces, state rows, and the graph all need to agree forever.

Fireline's better move is narrower:

- emit the right spans
- propagate the right trace context
- expose the right canonical ids on those spans
- let standard OTel tooling render the graph

That keeps the product small and the observation contract portable.

## What This Means For Durable Workflows

Durable subscribers and durable promises are where the observability story either holds or falls apart.

They create the exact kinds of waits and cross-process edges that make synthetic tracing schemes attractive:

- a passive wait may resolve much later from another process
- an active subscriber may retry across failures
- a webhook may succeed after the originating host has restarted
- an awakeable may suspend and resume across replay

Fireline's answer is not to invent a subscriber-specific trace system.

It is to keep the same trace continuity through those durable boundaries while keeping the durable stream as the product-state record. The trace explains the execution path; the stream explains the durable fact.

That split is why the system stays debuggable without becoming internally contradictory.

## What The User Gets

For an engineer adopting Fireline, this design buys a simpler mental model:

- state inspection comes from `fireline.db()` and the durable stream
- execution lineage comes from traces in the backend they already use
- retries, peer hops, approvals, and subscriber deliveries are visible without custom correlation glue
- there is no requirement to adopt a Fireline-specific observability product

That is a practical benefit, not a philosophical one. It means Fireline can fit into an existing OTLP environment instead of demanding a new one.

## When To Reach For Which Surface

Use durable state when you need durable truth:

- approval status
- prompt and tool output
- queue and completion state
- replayable workflow facts

Use traces when you need execution understanding:

- latency
- causality
- retries
- distributed joins
- boundary crossings into peers, webhooks, or other processes

If a question needs both, Fireline intends those surfaces to line up through canonical ids rather than through a third custom model.

## Relationship To Canonical Identifiers

This RFC is the observability follow-through to [RFC: ACP Canonical Identifiers](./canonical-identifiers.md).

Canonical ids answer "what is the work item?"

Observability answers "how did that work item move?"

Those two answers must stay compatible or the architecture stops making sense.

## References

- [RFC: ACP Canonical Identifiers](./canonical-identifiers.md)
- [RFC: Durable Subscribers](./durable-subscriber.md)
- [RFC: Durable Promises](./durable-promises.md)
- [Observation](../observation.md)
- [Proposal: Observability Integration](../../proposals/observability-integration.md)
