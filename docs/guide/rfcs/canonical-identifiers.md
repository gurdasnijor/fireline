# RFC: ACP Canonical Identifiers

> Status: design rationale
> Audience: engineers deciding whether to trust Fireline's built-in identity model or keep custom correlation and lineage glue around it

Fireline's architecture only works cleanly if the agent plane has one identity contract.

That contract is not "whatever ids Fireline happens to mint for convenience." It is ACP's own schema:

- `SessionId` for a session
- `RequestId` for a request
- `ToolCallId` for a tool invocation
- W3C trace context in ACP `_meta` for cross-hop lineage

Everything else is either a storage convenience or infrastructure bookkeeping.

That is the decision.

## The Problem This Solves

Systems start inventing ids for understandable reasons.

One layer needs a stable row key. Another wants to join related work across processes. A third needs to reconnect a callback to the thing that triggered it. Very quickly, a product that already has protocol identities grows a second identity system beside them.

That second system looks helpful for a while. Then it starts to rot:

- the same logical event has two names
- retries and callbacks pick different keys
- lineage needs bespoke stitching logic
- user-facing state leaks host-specific details
- every new feature has to decide which identity system is "real"

Fireline is explicitly choosing not to live with that drift.

## The Decision

Fireline standardizes the agent plane on canonical ACP identity only.

In practice, that means:

- a prompt-level fact is identified by `(sessionId, requestId)`
- a tool-level fact is identified by `(sessionId, toolCallId)`
- cross-session causality travels through `_meta.traceparent`, `_meta.tracestate`, and `baggage`
- host, runtime, node, and provider ids stay in the infrastructure plane

If a workflow cannot be described in those terms, Fireline treats it as infrastructure bookkeeping, not as agent-plane product identity.

That keeps one answer to the question "what work item is this?"

## Why Fireline Rejects Synthetic Agent-Plane IDs

The strongest version of this rule is simple:

If Fireline needs a made-up id to explain a prompt, approval, tool call, or lineage edge, the architecture is already off course.

The problem with synthetic ids is not that they are ugly. The problem is that they become semantic. Once they do, they start competing with the protocol's own identifiers.

That creates avoidable failure modes:

- an approval request has both a "real" request id and a Fireline request id
- a prompt has both an ACP request identity and a Fireline turn identity
- a peer hop has both trace context and a Fireline lineage pointer

At that point, integrations cannot tell which identifier to dedupe on, operators cannot tell which surface is authoritative, and new abstractions inherit old accidental seams.

Fireline rejects that entire class of problems by making the protocol's identifiers the only agent-plane truth.

## Why W3C Trace Context Owns Lineage

Lineage is the easiest place to accidentally invent a second system.

It is tempting to keep a table of parent-child edges or a Fireline-specific trace token because those structures are easy to query locally. But cross-session causality already has a standard representation: distributed tracing.

Fireline therefore treats lineage as trace propagation, not as a Fireline graph format.

That matters because the interesting questions are operational, not cosmetic:

- which session caused this peer call
- which prompt caused this webhook
- which approval led to this external action
- which wake event provisioned this runtime

Those are exactly the questions W3C trace context and span structure are built to answer.

So Fireline propagates `_meta.traceparent`, `_meta.tracestate`, and `baggage` instead of minting a bespoke lineage spine. The trace tree is the lineage.

## Plane Separation Is Part Of The Identity Model

Canonical identifiers only stay clean if the agent plane and the infrastructure plane stay separate.

The agent plane is what application developers and workflow logic should reason about:

- sessions
- prompts
- tool calls
- approvals
- chunks
- subscriber completions

The infrastructure plane is what operators and the host runtime should reason about:

- hosts
- runtimes
- nodes
- provider instances
- subscriber retry state
- deployment materialization details

Those planes are allowed to meet at provisioning time, because a runtime exists in order to host a session. After that, the session's identity is still just `SessionId`. Fireline does not keep re-injecting `host_key`, `runtimeId`, or similar infrastructure tokens into the user-facing state model.

This is not an aesthetic preference. It is how the product avoids turning operational implementation details into user-visible semantics.

## Why This Is A Prerequisite For Durable Workflows

Durable subscribers and durable promises both depend on stable completion identity.

If Fireline were still using synthetic prompt ids, hashed approval ids, or bespoke lineage pointers, the durable layer would inherit those seams and make them harder to remove later. A generalized durable substrate would then be built on top of transitional mistakes.

Canonical identifiers prevent that.

They give Fireline one trustworthy way to say:

- this approval belongs to this request
- this webhook delivery belongs to this tool call
- this awakeable resolves the same logical wait after restart
- this trace is the same causal chain across peer hops and external systems

That is why canonical ids are not just a cleanup pass. They are the base that makes the rest of the architecture coherent.

## What This Buys The User

For a product user or integrator, the value is straightforward:

- fewer Fireline-specific correlation rules to learn
- clearer interop with ACP-native tools and SDKs
- easier idempotence for retries and callbacks
- better trace continuity across system boundaries
- less risk that a future feature exposes a second naming scheme for the same work

The result is a system that is easier to trust because its public semantics line up with the protocol it already speaks.

## When To Lean On Fireline's Identity Model

Lean on the built-in model when you need to correlate user-facing work:

- approvals
- tool results
- webhook callbacks
- peer routing
- durable waits
- trace-aware observation

Keep extra ids local only when they are truly local:

- cache keys
- transport batching handles
- row storage keys that are pure concatenations of canonical ids
- operator-only runtime identifiers

That is the boundary Fireline is trying to hold.

## Relationship To The Rest Of The Architecture

Canonical identifiers are the identity layer.

On top of that:

- durable subscribers use canonical completion keys instead of bespoke delivery ids
- durable promises reuse those same keys as imperative workflow sugar
- observability follows W3C trace propagation instead of Fireline lineage tables
- hosted deployment stays honest about what is deployment identity versus runtime bookkeeping

If this layer drifts, everything above it gets less trustworthy.

If this layer stays strict, the rest of the architecture composes cleanly.

## References

- [Proposal: ACP Canonical Identifiers](../../proposals/acp-canonical-identifiers.md)
- [RFC: Durable Subscribers](./durable-subscriber.md)
- [RFC: Durable Promises](./durable-promises.md)
- [Approvals](../approvals.md)
- [Durable Subscribers Guide](../durable-subscriber.md)
