# Durable Subscriber Primitive

> Status: proposal
> Date: 2026-04-12
> Scope: Rust host driver, verification, TypeScript middleware surface

## TL;DR

Generalize the durable workflow pattern already proven by the approval gate into a host-side `DurableSubscriber` primitive.

This proposal assumes [acp-canonical-identifiers.md](./acp-canonical-identifiers.md) lands cleanly first.

Under that assumption:

- every subscriber filters agent-layer events using canonical ACP schema fields
- every completion key is composed only from `sacp::schema` identifier types
- every outbound side effect propagates W3C Trace Context through `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage`
- every subscriber runs in the infrastructure plane while reading and completing agent-plane work
- no subscriber mints its own semantic identifier

The approval gate correctness review in [approval-gate-correctness.md](../reviews/approval-gate-correctness.md) already proved the semantic substrate we need: suspend/resume durability, restart-safe completion behavior, timeout handling, concurrent isolation, and rebuild-race safety. The canonical-identifiers proposal supplies the missing external identity contract so the generalized abstraction does not freeze transitional seams into the API.

---

## 1. Alignment with Canonical Identifiers

This proposal is governed by [acp-canonical-identifiers.md](./acp-canonical-identifiers.md).

That document sets the acceptance bar:

- no synthetic ids
- no bespoke lineage stitching
- only ACP-schema identifiers in the agent plane
- W3C Trace Context propagated via ACP `_meta`
- infrastructure ids kept in the infrastructure plane

DurableSubscriber adopts that bar without exception.

Concretely:

1. `DurableSubscriber` does not introduce a new semantic identifier for events, completions, retries, dead letters, branches, or correlations.
2. Completion identity is always derived from canonical ACP identifiers already present in the matched event.
3. Storage row keys may serialize canonical key tuples for durable-stream convenience, but the semantic key remains typed until the final wire encoder.
4. Subscriber lineage uses `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage`, not a Fireline-specific correlation field.
5. The abstraction depends on the canonical-identifiers execution order. It is not safe to merge first and retrofit later.

## 2. Motivation

### 2.1 The substrate is already proven

`crates/fireline-harness/src/approval.rs` already demonstrates the durable workflow pattern:

1. emit an intent event to the agent stream
2. observe the stream live through durable-streams SSE
3. rebuild state from the log after restart
4. resume when the matching completion arrives

The review in [approval-gate-correctness.md](../reviews/approval-gate-correctness.md) confirmed that this substrate is semantically sound.

### 2.2 Multiple features want the same primitive

The same pattern appears in six categories:

| Feature | Input event | Output/completion | Canonical key |
|---|---|---|---|
| Approval gate | `permission_request` | `approval_resolved` | `PromptKey(SessionId, RequestId)` |
| Durable webhooks | prompt or tool event | `webhook_delivered` | `PromptKey` or `ToolKey` |
| Auto-approval | `permission_request` | `approval_resolved` | `PromptKey(SessionId, RequestId)` |
| Peer routing | prompt or tool event | `peer_call_delivered` or callee session start | `CrossSessionKey(SessionId, RequestId, SessionId)` |
| Wake timers | prompt-bound reminder | `timer_fired` | `PromptKey(SessionId, RequestId)` |
| External integrations | prompt/tool event | delivery/ack envelope | `PromptKey` or `ToolKey` |

Without a shared primitive, each feature reimplements:

- SSE live subscription
- replay/rebuild behavior
- completion dedupe
- timeout and retry decisions
- crash-safe side-effect sequencing

### 2.3 What generalization buys us

- one correctness story instead of six ad hoc ones
- one middleware surface instead of feature-specific wiring
- one verification target keyed by canonical ACP references
- one host-side place to manage retries, dead letters, and observability

---

## 3. Canonical Contract

### 3.1 DurableSubscriber state machine

The generic loop is:

1. read agent-plane event from `state/session/{session_id}`
2. filter by canonical ACP fields
3. derive `CompletionKey` from canonical ACP identifiers in the event
4. check whether that completion already exists
5. run the handler or wait for an external completer
6. append the domain completion back to the agent-plane stream
7. keep retry/dead-letter bookkeeping in an infrastructure-plane subscriber stream

### 3.2 Trait shape

```rust
use sacp::schema::{RequestId, SessionId, ToolCallId};

pub trait DurableSubscriber: Send + Sync {
    type Event: DeserializeOwned + Send;
    type Completion: Serialize + Send;

    /// Infrastructure-facing name for metrics, config lookup, and admin UX.
    /// This is not an agent-layer identifier.
    fn name(&self) -> &str;

    /// Match filter over a typed agent-plane envelope.
    fn matches(&self, envelope: &StreamEnvelope) -> Option<Self::Event>;

    /// Completion identity derived only from canonical ACP identifiers
    /// already present in the event.
    fn completion_key(&self, event: &Self::Event) -> CompletionKey;

    /// Has a completion with the same canonical key already been observed?
    fn is_completed(&self, event: &Self::Event, log: &[StreamEnvelope]) -> bool;

    /// Execute or wait for the completion path.
    async fn handle(&self, event: Self::Event) -> HandlerOutcome<Self::Completion>;

    fn retry_policy(&self) -> RetryPolicy {
        RetryPolicy::default()
    }
}

pub enum HandlerOutcome<C> {
    /// Subscriber actively produced the completion.
    Completed(C),
    /// Subscriber is passive and waits for another writer to append completion.
    Passive,
    /// Retryable failure; bookkeeping lives in the infrastructure plane.
    RetryTransient(anyhow::Error),
    /// Permanent failure; dead-letter bookkeeping lives in the infrastructure plane.
    Failed(anyhow::Error),
}
```

### 3.3 Type-Enforced Key Composition

`CompletionKey` is not an opaque string.

```rust
use sacp::schema::{RequestId, SessionId, ToolCallId};

pub enum CompletionKey {
    PromptKey(SessionId, RequestId),
    ToolKey(SessionId, ToolCallId),
    CrossSessionKey(SessionId, RequestId, SessionId),
}
```

Rules:

- every variant is composed only from canonical ACP types
- no variant accepts `String`
- no variant accepts a counter, random token, or derived payload fingerprint
- serialization to a durable-stream row key happens only at the storage edge

This is the main compile-time protection the proposal adds. A subscriber implementation cannot accidentally return an invented identifier because the trait has nowhere to put one.

### 3.4 Event filtering

Subscribers filter on typed ACP fields, not on Fireline-private seams:

- `type`
- `session_id: SessionId`
- `request_id: RequestId`
- `tool_call_id: ToolCallId`
- `_meta.traceparent`
- `_meta.tracestate`
- `_meta.baggage`

`StreamEnvelope` in this proposal therefore means "typed agent-plane envelope after canonical-id normalization", not an untyped JSON blob.

### 3.5 Plane placement

The `DurableSubscriberDriver` lives in the infrastructure plane.

- it runs inside the always-on Fireline host process
- it reads agent-plane streams such as `state/session/{session_id}` through durable-streams SSE
- it writes domain completions back to the agent-plane stream when agent-layer semantics care about the result
- it stores subscriber config, retry state, and dead-letter bookkeeping in an infrastructure stream such as `subscribers:tenant-{id}`

This keeps the planes separate:

- agent plane contains agent-facing state transitions
- infrastructure plane contains subscriber driver mechanics

The driver may observe both planes, but it does not project infrastructure rows onto agent-layer entities.

---

## 4. Rust Design

### 4.1 Driver shape

```rust
pub struct DurableSubscriberDriver {
    subscribers: Vec<Arc<dyn DurableSubscriber<Event = Value, Completion = Value>>>,
    infra_stream: DurableStream,
    agent_reader_factory: Arc<dyn AgentStreamReaderFactory>,
    agent_completion_producer_factory: Arc<dyn AgentCompletionProducerFactory>,
}
```

Responsibilities:

- subscribe to relevant agent streams with SSE
- dispatch matched events to subscriber implementations
- check for existing completions by `CompletionKey`
- append domain completions to the correct agent stream
- persist retry attempts and dead-letter state in the infrastructure stream
- expose health and progress metrics keyed by subscriber `name()`

### 4.2 Trace propagation

Every subscriber side effect must propagate canonical trace context.

Rules:

1. Extract `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage` from the source event.
2. When the side effect is ACP-shaped, write those fields back into outbound `_meta`.
3. When the side effect is generic HTTP, also inject the corresponding W3C headers so the receiver joins the same trace.
4. When the side effect produces a completion envelope, copy the same trace context into that envelope's `_meta`.

This is mandatory for:

- webhook POSTs
- peer calls
- Slack/email/GitHub outbound calls
- any future subscriber that crosses process or host boundaries

### 4.3 Relationship to `approval.rs`

Once canonical identifiers land, the approval gate becomes the first passive subscriber.

```rust
pub struct ApprovalGateSubscriber;

impl DurableSubscriber for ApprovalGateSubscriber {
    type Event = PermissionRequest;
    type Completion = ApprovalResolved;

    fn name(&self) -> &str {
        "approval_gate"
    }

    fn matches(&self, envelope: &StreamEnvelope) -> Option<Self::Event> {
        match_permission_request(envelope)
    }

    fn completion_key(&self, event: &Self::Event) -> CompletionKey {
        CompletionKey::PromptKey(event.session_id.clone(), event.request_id.clone())
    }

    fn is_completed(&self, event: &Self::Event, log: &[StreamEnvelope]) -> bool {
        log.iter().any(|env| {
            matches_approval_resolved(env, &event.session_id, &event.request_id)
        })
    }

    async fn handle(&self, _event: Self::Event) -> HandlerOutcome<Self::Completion> {
        HandlerOutcome::Passive
    }
}
```

Key point: the generalization uses the canonical permission request id carried by ACP. It does not preserve any transitional id-generation strategy.

---

## 5. Use Cases

### 5.1 ApprovalGateSubscriber

- event: `permission_request`
- completion: `approval_resolved`
- key: `PromptKey(SessionId, RequestId)`
- mode: passive
- trace: completion envelope carries the same `_meta` trace context as the source request

This is the reference case because the approval review already proved the behavior semantically.

### 5.2 WebhookSubscriber

- event: prompt-level or tool-level agent-plane event
- completion: `webhook_delivered`
- key:
  - prompt-level: `PromptKey(SessionId, RequestId)`
  - tool-level: `ToolKey(SessionId, ToolCallId)`
- mode: active

Webhook side effects must:

- inject `traceparent`, `tracestate`, and `baggage` as HTTP headers
- include the same values in payload `_meta` when the body is ACP-shaped JSON
- write delivery completion back to the agent stream

### 5.3 AutoApproveSubscriber

- event: `permission_request`
- completion: `approval_resolved`
- key: `PromptKey(SessionId, RequestId)`
- mode: active

This is the active mirror of the approval gate. It watches the same event shape and can auto-resolve according to policy without inventing a second approval identity.

### 5.4 PeerCallSubscriber

- event: outbound prompt or tool event on the caller side
- completion: peer delivery acknowledgment or callee session start
- key: `CrossSessionKey(caller_session_id, caller_request_id, callee_session_id)`
- mode: active

Trace rule:

- outbound peer ACP request must propagate `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage`
- callee-side completion keeps the same trace lineage

### 5.5 WakeTimerSubscriber

Two cases exist:

1. agent-bound timer
   - event: prompt/request asking for a deferred wake
   - completion: `timer_fired`
   - key: `PromptKey(SessionId, RequestId)`
2. infrastructure-only cluster timer
   - out of first cut for this proposal
   - stays entirely in the infrastructure plane until there is a canonical agent binding

The first cut of DurableSubscriber should only model the agent-bound case, because the type-enforced `CompletionKey` enum intentionally covers ACP-bound identity shapes only.

### 5.6 ExternalIntegrationSubscriber

- event: prompt-level or tool-level agent event
- completion: provider-specific delivery acknowledgment
- key: `PromptKey` or `ToolKey`
- mode: active

Slack, email, GitHub, and similar integrations all follow the same rule as webhooks:

- canonical ACP key on the input side
- W3C Trace Context on the outbound side
- completion written back to the agent stream
- retry/dead-letter state stored in the infrastructure stream

---

## 6. TypeScript Middleware Surface

### 6.1 Example

```typescript
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { webhook } from '@fireline/client/middleware'
import type { SessionId, RequestId, ToolCallId } from '@agentclientprotocol/sdk'

compose(
  sandbox({ provider: 'local' }),
  middleware([
    webhook({
      name: 'audit-to-slack',
      events: ['permission_request', 'tool_call_completed'],
      url: 'https://hooks.slack.com/...',
      keyBy: 'session_request',
      headers: { 'X-Signature': { ref: 'secret:slack-signing-key' } },
      retry: { maxAttempts: 5, initialBackoffMs: 1000 },
    }),
  ]),
  agent(['claude-sonnet-4-6']),
)
```

### 6.2 Key strategy surface

The middleware API exposes declarative keying only:

```typescript
type SubscriberKeyStrategy =
  | 'session'
  | 'session_request'
  | 'session_tool_call'
  | 'cross_session'
```

Mapping:

- `'session'` means the subscriber expects only a `SessionId`
- `'session_request'` maps to `CompletionKey::PromptKey`
- `'session_tool_call'` maps to `CompletionKey::ToolKey`
- `'cross_session'` maps to `CompletionKey::CrossSessionKey`

No custom string key is allowed in userland middleware.

### 6.3 Type discipline

The TS surface must type ACP identifiers through `@agentclientprotocol/sdk`, not plain `string`.

That means:

- subscriber event payload types use branded ACP identifiers
- helper functions such as `appendApprovalResolved(...)` type `SessionId` and `RequestId`
- middleware specs cannot accept custom correlation ids

### 6.4 Trace propagation in middleware

The middleware surface does not ask the user for trace fields.

Instead:

- source `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage` are automatically propagated
- outbound webhook HTTP requests receive W3C trace headers
- ACP-shaped outbound payloads mirror the same values in `_meta`

---

## 7. Verification

### 7.1 Preconditions from the canonical-id work

DurableSubscriber verification depends on the canonical-identifiers invariants.

At minimum, the TLA and test plan should assume:

- `AgentLayerIdentifiersAreCanonical`
- `AgentLayerRowsExcludeInfrastructureIds`
- `TraceContextFlowsThroughMeta`

If those invariants are not already established, subscriber verification is proving the wrong surface.

### 7.2 Subscriber invariants

Once canonical ids are in place, subscriber invariants key directly by canonical tuples:

- `PromptKey == <<session_id, request_id>>`
- `ToolKey == <<session_id, tool_call_id>>`
- `CrossSessionKey == <<caller_session_id, caller_request_id, callee_session_id>>`

No separate matched-event id or completion id is introduced.

Core invariants:

1. every matched canonical key is eventually completed or dead-lettered
2. at most one completion exists per `(subscriber_name, CompletionKey)`
3. replay from any offset preserves the same completed key set
4. retry and dead-letter bookkeeping never leaks into agent-layer rows
5. completion envelopes preserve source trace context

### 7.3 TLA+ extension shape

The TLA model should add:

- subscriber config and retry state in the infrastructure plane
- agent-plane completion events keyed by canonical tuples
- subscriber actions that match, complete, retry, or dead-letter by canonical `CompletionKey`

The model should not add:

- a new synthetic event id
- a new synthetic completion id
- a parallel lineage table

### 7.4 Validation against the approval proof

The approval correctness review already gives us the semantic regression bar:

- suspend and resume across crash
- timeout behavior
- concurrent isolation
- rebuild race with live resolution

The generalized subscriber implementation should preserve those behaviors when the approval gate is re-expressed against canonical ACP `RequestId`.

---

## 8. Implementation Plan

### 8.1 Prerequisite gates

Subscriber generalization cannot start before these canonical-id milestones:

1. canonical-identifiers Phase 2 is landed
   - the approval flow is operating on canonical request references rather than transitional identity seams
2. canonical-identifiers Phase 5 is landed
   - `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage` propagate end to end

If either prerequisite is missing, DurableSubscriber risks freezing pre-canonical assumptions into the abstraction.

### 8.2 Phase 1: typed subscriber core

1. add `CompletionKey`
2. add `DurableSubscriber`
3. add a typed agent-plane envelope decoder using `sacp::schema` types
4. add infrastructure-plane retry/dead-letter store

Acceptance:

- no trait surface accepts opaque string ids
- no completion key is representable without canonical ACP types

### 8.3 Phase 2: approval gate extraction

1. keep `ApprovalGateComponent` as the topology component
2. move its internals behind `ApprovalGateSubscriber`
3. preserve the proven behavior from the review

Acceptance:

- approval path uses canonical `RequestId`
- replay, timeout, and rebuild-race tests remain green

### 8.4 Phase 3: active subscribers

1. implement `WebhookSubscriber`
2. implement `AutoApproveSubscriber`
3. implement external-integration subscribers on the same abstraction

Acceptance:

- outbound HTTP requests inject W3C trace headers
- completion envelopes write back to the agent stream
- retry/dead-letter state stays in the infrastructure stream

### 8.5 Phase 4: peer and timer subscribers

1. implement `PeerCallSubscriber`
2. implement prompt-bound `WakeTimerSubscriber`
3. defer infrastructure-only timers until a clean infra-only contract is written

### 8.6 Phase 5: verification and docs

1. extend TLA with canonical `CompletionKey` tuples
2. add end-to-end tests after the canonical-id execution is complete
3. document `webhook()`, `autoApprove()`, and approval-gate-as-subscriber

---

## 9. What This Does Not Solve

- subscriber business logic is still application policy; the primitive only makes the durability contract explicit
- infrastructure-only timers without an agent binding are deferred
- receiver-side dedupe for external systems is still the receiver's responsibility
- cross-subscriber coordination beyond stream-visible completions is not a first-cut feature
- this proposal does not relax plane separation by letting subscriber bookkeeping leak into agent-layer entities

---

## 10. Open Questions

1. Resolution sources for passive approval flows.

   When the approval gate becomes a passive DurableSubscriber, who writes `approval_resolved`?

   - `agent.resolvePermission()` when the app owns the agent process
   - `appendApprovalResolved(streamUrl, ...)` for external dashboards, webhooks, or automation
   - another `DurableSubscriber`, such as `AutoApproveSubscriber`

   All three should remain valid. The primitive must stay compatible with in-process and out-of-process resolution.

2. Whether `webhook_delivered` belongs in every agent stream or only when the application explicitly opts into observing delivery outcomes.

3. Whether subscriber configuration should live only in static topology or also support infrastructure-plane live reload later.

---

## 11. Validation against Canonical Identifiers

- [ ] Every `CompletionKey` variant is composed only of `sacp::schema` types
- [ ] Every subscriber implementation in the proposal uses canonical ACP types for event fields
- [ ] No subscriber hashes payloads, mints UUIDs, or derives keys from anything other than ACP identifiers
- [ ] Webhook and notification outbound calls inject `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage`
- [ ] Subscriber infrastructure state for retry and dead-letter lives in the infrastructure plane, not on agent-layer rows
- [ ] The TypeScript middleware surface types identifiers via `@agentclientprotocol/sdk`

---

## 12. References

- [acp-canonical-identifiers.md](./acp-canonical-identifiers.md)
- [approval-gate-correctness.md](../reviews/approval-gate-correctness.md)
- `crates/fireline-harness/src/approval.rs`
- `docs/proposals/webhook-support.md`
- `verification/spec/managed_agents.tla`
