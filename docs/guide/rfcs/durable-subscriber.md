# RFC: Durable Subscribers

> Status: design rationale
> Audience: engineers deciding whether to build on Fireline's durable-subscriber substrate or keep an integration as a one-off adapter

Fireline did not introduce durable subscribers because the middleware catalog needed a nicer name.

It introduced them because the same reliability problem kept showing up in different clothes:

- approvals need a durable wait
- webhooks need durable delivery
- Telegram needs durable fan-out to a human surface
- auto-approve needs the same completion semantics as manual approval
- wake and deployment flows need durable event-to-completion progression

Once approval proved that the pattern works, the lower-risk move was to generalize it into one substrate instead of rebuilding the same crash, replay, retry, and correlation logic for every feature.

That is the core decision behind `DurableSubscriber`.

## The Decision

Fireline standardizes long-lived event handling on one durable substrate:

- observe an agent-plane event
- derive a stable completion identity from that event
- survive restart and replay
- either wait for someone else to complete the work or perform the side effect directly
- record forward progress durably

The substrate has two modes:

- passive: Fireline writes the durable wait point and another actor completes it later
- active: Fireline performs the delivery or action itself and records the outcome durably

Those are not two architectures. They are two responsibility boundaries on the same architecture.

## Why This Had To Become A Substrate

Approval was the proof.

The approval gate already demonstrated the important properties Fireline wanted:

- the request lives on the durable stream
- a restart does not lose the wait
- an external actor can resolve the request later
- replay converges on the same result instead of inventing a second workflow

Once that existed, the right question was no longer "how do we implement webhooks?" or "how do we add Telegram?"

The right question became:

"How many times do we want to solve durable event handling?"

Fireline's answer is: once.

That decision matters because the hard parts are not the per-feature formatting details. The hard parts are:

- restart-safe progress
- completion deduplication
- replay equivalence
- retry and dead-letter behavior
- trace continuity across process boundaries

Those concerns belong in a substrate, not copied into five integrations.

## One Substrate, Many Profiles

Durable subscribers are the common model behind multiple user-facing behaviors:

| Profile | Mode | Why it belongs on the same substrate |
| --- | --- | --- |
| Approval gate | Passive | writes a durable request and waits for matching completion |
| Webhook delivery | Active | sends an event outward and tracks completion/retry durably |
| Telegram delivery | Active | same delivery problem, different human-facing transport |
| Auto-approve | Active | completes the same approval key automatically |
| Peer routing | Active | forwards work across an agent boundary with the same durability expectations |
| Wake and deployment flows | Active | durable event-to-provision progression keyed by deployment identity |

The point is not that these features look the same in product screenshots.

The point is that they all need the same underlying guarantees: derive the logical work item, survive crashes, avoid double-winning the same completion, and rebuild correctly from the stream.

## Passive And Active Are One System

The passive/active split is a design convenience, not a fracture line.

Passive mode is for cases where Fireline should preserve the wait but not own the decision. Approval is the clearest example: Fireline records the request, but a human, dashboard, bot, or another process decides how it resolves.

Active mode is for cases where Fireline should deliver or act automatically. Webhook posting, Telegram cards, auto-approval, peer forwarding, and always-on wake flows all fit here.

What stays the same across both modes:

- matching happens against agent-plane events
- completion identity comes from the event, not from an invented subscriber token
- restart and replay use the durable stream as the source of truth
- forward progress is recorded durably before the system forgets the work

That is why Fireline treats passive and active as profiles on one substrate instead of separate products.

## Completion Keys Are Derived, Not Invented

This is one of the most important architectural choices in the design.

Durable subscribers do not mint a new semantic id for every delivery, wait, or callback. They derive completion identity from canonical identifiers already present in the event itself.

In practice, that usually means:

- prompt-shaped work resolves once per `(sessionId, requestId)`
- tool-shaped work resolves once per `(sessionId, toolCallId)`
- deployment and wake flows resolve against the deployment/session identity they already carry

Why Fireline insists on this:

- retries can target the same logical work item
- downstream systems can dedupe without Fireline-specific correlation hacks
- different profiles can interoperate because they agree on the meaning of "the same work"
- the architecture does not freeze a second, subscriber-only identity layer into the product

This is what makes durable subscribers composable instead of merely reusable.

## At-Least-Once And Cursor Monotonicity Are Features

Durable subscribers are intentionally at-least-once for active delivery.

That is not a compromise hidden behind friendly language. It is the honest semantics of crash-safe external delivery. If Fireline sends a webhook and dies before it durably records success, it must be allowed to try again after restart.

Fireline therefore chooses a design that is defensible:

- stable logical completion keys for idempotence
- retries that are explicit instead of magical
- dead-letter handling when delivery does not converge
- cursor movement that only goes forward once the matched event has been handled

Cursor monotonicity matters just as much as retries.

If a subscriber could jump its cursor ahead before durable handling was settled, later events could appear "processed" while earlier work was still unresolved. Fireline refuses that ambiguity. The cursor only advances when it is safe to say the substrate has made durable forward progress.

Together, at-least-once delivery and monotonic cursor movement make the system restart-safe in a way one-off adapters usually are not.

## Trace Propagation Is A Substrate Property

Trace propagation is not a webhook-specific nicety. It is part of what durable subscribers are for.

The moment Fireline turns an event into an external side effect, it is crossing a boundary where operators need lineage:

- which prompt caused this webhook
- which approval request produced this Telegram card
- which peer hop continued this trace
- which wake action provisioned this runtime

That is why W3C trace context lives in the substrate itself. The outbound side effect and the eventual completion both carry the same `_meta.traceparent`, `_meta.tracestate`, and `baggage` continuity the original event had.

If trace propagation were left to each profile, it would drift, and the architecture would stop being coherent at the exact moment work left the host process.

## Durable Subscribers Vs One-Off Adapters

Use a durable subscriber when any of these are true:

- losing the work on restart would be wrong
- the event crosses a process, host, or human boundary
- retries need to be deliberate and visible
- multiple integrations should share one correctness story
- you need the completion to line up with a stable logical work item

Keep a one-off adapter only when all of these are true:

- the effect is local and ephemeral
- there is no durable wait or external completion
- replay does not need to reconstruct the work
- losing the in-flight action on process death is acceptable

That is the real decision test. Durable subscribers are not for every callback in a codebase. They are for the moment an event becomes durable operational work.

## Relationship To Durable Promises

Durable promises do not replace this substrate.

They are the workflow-facing, imperative spelling of passive durable-subscriber behavior. If durable subscribers answer "how does Fireline handle this class of event durably?", durable promises answer "how does workflow code pause on that same durable fact without becoming infrastructure code?"

That layering is intentional:

- durable subscribers are the substrate
- durable promises are sugar over the passive half of that substrate

## References

- [Durable Subscribers Guide](../durable-subscriber.md)
- [RFC: Durable Promises](./durable-promises.md)
- [Proposal: Durable Subscriber Primitive](../../proposals/durable-subscriber.md)
- [Proposal: Durable Subscriber Verification](../../proposals/durable-subscriber-verification.md)
