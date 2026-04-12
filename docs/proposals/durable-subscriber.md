# Durable Subscriber Primitive

> **Status:** proposal
> **Scope:** Rust harness + verification spec + TypeScript middleware surface
> **Date:** 2026-04-12

## TL;DR

Extract the pattern already implemented in `crates/fireline-harness/src/approval.rs` into a generalized **DurableSubscriber** primitive. One abstraction collapses six concrete features that today either exist as one-offs or are ad-hoc:

| Feature | Today | After |
|---|---|---|
| Approval gate | One-off in `approval.rs` | `DurableSubscriber<PermissionRequest, ApprovalResolved>` |
| Durable webhooks | Doesn't exist; examples inline HTTP servers | `DurableSubscriber<StreamEvent, WebhookDelivered>` |
| Auto-approval policy | Doesn't exist | `DurableSubscriber<PermissionRequest, ApprovalResolved>` |
| Cross-agent routing | Partial via `peer_mcp` | `DurableSubscriber<PeerCall, PeerCallAcked>` |
| Scheduled wake / deferred work | Doesn't exist | `DurableSubscriber<WakeTimer, TimerFired>` |
| External integration (Slack, email, GitHub) | Doesn't exist | `DurableSubscriber<StreamEvent, NotificationSent>` |

The substrate is already present — durable-streams SSE readers, `rebuild_from_log`, idempotent stream appends via `CommitTuple` dedupe. This proposal extracts the state machine, documents the contract, adds verifiable correctness properties to `verification/spec/managed_agents.tla`, and introduces a TypeScript middleware surface so subscribers can be declared in compose specs.

---

## 1. Motivation

### 1.1 The pattern is already proven

`crates/fireline-harness/src/approval.rs` demonstrates a concrete durable subscriber:

1. **Emit intent** (`emit_permission_request` at `approval.rs:254`) — writes `permission_request` to the durable state stream
2. **Observe live** (`wait_for_approval` at `approval.rs:294`) — opens a `LiveMode::Sse` reader, blocks until matching `approval_resolved` appears
3. **Resume on restart** (`rebuild_from_log` at `approval.rs:176`) — on `session/load`, replays the stream from offset 0 to reconstruct in-memory state (pending reason, approved flag)
4. **Act after match** — forwards the ACP prompt call to the agent

This is a complete event-sourced state machine: pending → resolved → acted → completed. The stream is the source of truth. Every step can be interrupted and restarted without data loss.

### 1.2 Every other "durable workflow" feature wants the same pattern

The backend Node subscriber failure-mode analysis surfaced this clearly: any logic that reacts to stream events and performs side effects needs

- at-least-once delivery (replay on restart)
- idempotent side effects (or effects recorded on the stream so replay can skip already-completed work)
- offset persistence / completion markers
- bounded retry with dead-letter semantics

Building each feature (webhooks, auto-approval, Slack notifiers) as a one-off reinvents this state machine every time. Today the approval gate is the one place that got it right — in Rust, tightly coupled to ACP's prompt proxy path. Everything else falls back to application-level polling or is missing entirely.

### 1.3 What generalization buys us

1. **Six features collapse into one primitive** — approvals, webhooks, auto-approvers, peer routing, scheduled wake, external integrations
2. **Verifiable correctness once** — the TLA+ spec proves the pattern, not every instance
3. **Composable surface** — subscribers are declared like other middleware, not coded ad-hoc
4. **Host-side durability** — subscribers run inside the always-on fireline host, not in ephemeral user processes
5. **No new substrate** — uses durable-streams SSE + idempotent appends, which already exist

---

## 2. The DurableSubscriber contract

### 2.1 Abstract state machine

```
           ┌──────────────┐
           │ stream event │
           └──────┬───────┘
                  │
                  ▼
           ┌──────────────┐
           │   matches?   │──── no ─── (ignore)
           └──────┬───────┘
                  │ yes
                  ▼
           ┌──────────────┐
           │  completed?  │──── yes ── (skip — replay safety)
           └──────┬───────┘
                  │ no
                  ▼
           ┌──────────────┐
           │   handler    │ ── err ── retry (bounded)
           └──────┬───────┘
                  │ ok
                  ▼
           ┌──────────────┐
           │   append     │  completion envelope
           │  completion  │  (deterministic key prevents double-append)
           └──────────────┘
```

Every transition is observable on the stream. `completed?` check and `append completion` are both deterministic over the stream — same input, same output. This makes replay trivially correct: the subscriber can start fresh with no local state and reconstruct "what's left to do" from the log alone.

### 2.2 Key invariants

Let `Matched(log)` = set of events in the log matching the subscriber's filter. Let `Completed(log)` = set of completion envelopes. Let `Pending(log) = Matched(log) \ Completed(log)` (by matched-event id).

The subscriber guarantees:

- **Progress:** `Pending(log)` decreases monotonically over time (modulo bounded retry state).
- **Completeness:** eventually, `Pending(log) = {}` or every outstanding event has exhausted its retry budget and a dead-letter envelope is present.
- **Idempotence:** replaying any prefix of the log produces the same set of completion envelopes.
- **No lost events:** every matched event is either completed or dead-lettered; nothing is silently dropped.
- **No duplicate completions:** for any matched event, at most one completion envelope exists (deduplication via deterministic stream keys).

### 2.3 Contract obligations on handlers

A handler's side effects must be one of:
- **Idempotent by construction** (e.g. `PUT` requests with deterministic resource IDs, stream appends with deterministic keys)
- **Gated by a completion check on the stream** (e.g. look for `notification_sent` envelope before sending Slack message)
- **Acceptable to repeat** (e.g. audit log entries — dedup happens at the sink)

Handlers that violate this will duplicate side effects on crash-restart. The framework cannot hide this — it's an application-level concern. The subscriber primitive makes the hook point clear so this constraint is documented rather than discovered.

---

## 3. Rust API design

### 3.1 The trait

```rust
/// A durable subscriber over the state stream.
///
/// Filters matching events, invokes a handler, and records completion.
/// Replay-safe because every decision is derived from the stream.
pub trait DurableSubscriber: Send + Sync {
    /// Event type the subscriber extracts from stream envelopes.
    type Event: DeserializeOwned + Send;

    /// Completion envelope written after successful handling.
    type Completion: Serialize + Send;

    /// Name used in tracing and for the completion stream key.
    fn name(&self) -> &str;

    /// Match filter. Return `Some(event)` if this stream envelope is relevant.
    fn matches(&self, envelope: &StreamEnvelope) -> Option<Self::Event>;

    /// Deterministic completion key. Given the same event, returns the same key.
    /// The key ensures idempotent completion — two subscribers seeing the
    /// same event will write to the same slot.
    fn completion_key(&self, event: &Self::Event) -> String;

    /// Is this event already completed? Checked against the completion stream.
    fn is_completed(&self, event: &Self::Event, completion_log: &[StreamEnvelope]) -> bool;

    /// The side-effecting handler. May return retry-on-error.
    async fn handle(&self, event: Self::Event) -> HandlerOutcome<Self::Completion>;

    /// Retry policy. Default: exponential backoff, max 5 attempts, then dead-letter.
    fn retry_policy(&self) -> RetryPolicy { RetryPolicy::default() }
}

pub enum HandlerOutcome<C> {
    /// Handler succeeded. Append this completion envelope.
    Completed(C),
    /// Transient failure. Retry per the retry policy.
    RetryTransient(anyhow::Error),
    /// Permanent failure. Dead-letter this event; do not retry.
    Failed(anyhow::Error),
}
```

### 3.2 The driver

A single `DurableSubscriberDriver` component hosts any number of subscribers:

```rust
pub struct DurableSubscriberDriver {
    subscribers: Vec<Arc<dyn DurableSubscriber<Event = Value, Completion = Value>>>,
    stream_url: String,
    state_producer: Producer,
    retry_store: Arc<dyn RetryStore>,
}
```

The driver owns:
- The single SSE reader on the state stream (shared across subscribers)
- Dispatch to each subscriber's `matches` filter
- Retry orchestration (`RetryStore` persists attempt state on the stream so retry schedule survives restart)
- Completion append via `state_producer.append_json(completion_envelope)`

It registers as a topology component (`"durable_subscriber"`) so it participates in the normal conductor lifecycle.

### 3.3 Relationship to `approval.rs`

The existing `ApprovalGateComponent` becomes an **implementation of** `DurableSubscriber`:

```rust
impl DurableSubscriber for ApprovalGateSubscriber {
    type Event = PermissionRequest;
    type Completion = ApprovalResolved;

    fn name(&self) -> &str { "approval_gate" }

    fn matches(&self, env: &StreamEnvelope) -> Option<Self::Event> {
        if env.kind == "permission_request" {
            serde_json::from_value(env.value.clone()).ok()
        } else { None }
    }

    fn completion_key(&self, e: &Self::Event) -> String {
        format!("{}:{}:resolved", e.session_id, e.request_id)
    }

    fn is_completed(&self, e: &Self::Event, log: &[StreamEnvelope]) -> bool {
        log.iter().any(|env|
            env.kind == "approval_resolved" &&
            env.key == self.completion_key(e)
        )
    }

    async fn handle(&self, e: Self::Event) -> HandlerOutcome<Self::Completion> {
        // The approval gate's handle() blocks until an external party appends
        // the resolution. It's a passive handler — completion comes from
        // outside. This is a distinct mode from active handlers (webhooks).
        HandlerOutcome::Passive
    }
}
```

This introduces a distinction:

- **Active subscribers** have `handle()` return a completion. Examples: webhook dispatcher, Slack notifier, auto-approval policy.
- **Passive subscribers** wait for an external party to append the completion. The approval gate today is passive — the completion is appended by a dashboard, Slackbot, or human operator via `agent.resolvePermission()`.

The driver handles both. A passive subscriber doesn't invoke `handle()` — it just tracks `Pending(log)` for observability. An active subscriber drives `handle() → append completion` on each pending event.

### 3.4 Webhook subscriber — a concrete instance

```rust
pub struct WebhookSubscriber {
    name: String,
    filter: EventFilter,
    url: String,
    headers: Vec<(String, CredentialRef)>,
    resolver: Arc<dyn CredentialResolver>,
}

impl DurableSubscriber for WebhookSubscriber {
    type Event = StreamEnvelope;
    type Completion = WebhookDelivered;

    fn matches(&self, env: &StreamEnvelope) -> Option<Self::Event> {
        self.filter.matches(env).then(|| env.clone())
    }

    fn completion_key(&self, e: &Self::Event) -> String {
        format!("webhook:{}:delivered:{}", self.name, e.logical_id)
    }

    fn is_completed(&self, e: &Self::Event, log: &[StreamEnvelope]) -> bool {
        log.iter().any(|env| env.key == self.completion_key(e))
    }

    async fn handle(&self, e: Self::Event) -> HandlerOutcome<Self::Completion> {
        let headers = self.resolve_headers().await?;
        match http_post(&self.url, &e.value, headers).await {
            Ok(resp) if resp.status().is_success() => HandlerOutcome::Completed(
                WebhookDelivered {
                    logical_id: e.logical_id,
                    delivered_at_ms: now_ms(),
                    response_status: resp.status().as_u16(),
                }
            ),
            Ok(resp) if resp.status().is_server_error() =>
                HandlerOutcome::RetryTransient(anyhow!("5xx: {}", resp.status())),
            Ok(resp) =>
                HandlerOutcome::Failed(anyhow!("4xx: {}", resp.status())),
            Err(e) => HandlerOutcome::RetryTransient(e.into()),
        }
    }
}
```

---

## 4. TypeScript middleware surface

### 4.1 Declaration

Subscribers are declared in compose specs, matching the pattern used for `attachTools`, `secretsProxy`, etc.:

```typescript
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, webhook, autoApprove } from '@fireline/client/middleware'

export default compose(
  sandbox({ ... }),
  middleware([
    trace(),
    webhook({
      name: 'audit-to-slack',
      events: ['permission_request', 'approval_resolved'],
      url: 'https://hooks.slack.com/services/...',
      headers: { 'X-Signature': { ref: 'secret:slack-signing-key' } },
      retry: { maxAttempts: 5, initialBackoffMs: 1000 },
    }),
    autoApprove({
      name: 'read-only-approver',
      policy: { match: { kind: 'prompt_contains', needle: 'read' }, action: 'allow' },
    }),
  ]),
  agent([...]),
)
```

Both `webhook()` and `autoApprove()` map to the same `durable_subscriber` topology component with different subscriber configs. The topology registry in `host_topology.rs` resolves them to concrete `DurableSubscriber` impls.

### 4.2 Observation

Because subscribers append completion envelopes to the stream, they are observable via `fireline.db()` exactly like any other stream event. A webhook delivery log falls out for free:

```typescript
const delivered = useLiveQuery(q =>
  q.from({ d: db.webhookDeliveries }).where(({ d }) => d.subscriberName === 'audit-to-slack')
)
// UI shows delivery history, response codes, retry attempts — all from the stream.
```

New collections added to `@fireline/state` schema: `webhookDeliveries`, `deadLetters`, `subscriberProgress`. These are materialized views over the existing state stream — no new substrate.

---

## 5. Verification

### 5.1 Current TLA+ coverage

`verification/spec/managed_agents.tla` already models:

- **Session append-only semantics** (`SessionAppendOnly`, line 755)
- **Idempotent append via CommitTuple** (`SessionScopedIdempotentAppend`, line 769)
- **Approval request/resolve with history** (`RequestApproval`, `ResolveApproval`, lines 361 and 400)
- **Release gated by resolution** (`HarnessSuspendReleasedOnlyByMatchingApproval`, line 785)
- **Durability across runtime death** (`SessionDurableAcrossRuntimeDeath`, line 763)
- **Replay semantics** (`SessionReplayFromOffsetIsSuffix`, line 759)

This is most of the machinery we need. The approval gate is already specified in terms of events on the session log. Generalizing to `DurableSubscriber` means lifting the specific `permission_requested` / `approval_resolved` kinds to an abstract `MatchedEvent` / `CompletionEnvelope` pair.

### 5.2 Proposed TLA+ extensions

Add the following to `managed_agents.tla`:

```tla
CONSTANTS
  SubscriberNames,           \* symbolic subscriber identifiers
  CompletionKeys             \* deterministic completion key space

EventKinds ==
  \* existing kinds plus:
  {
    ...,
    "subscriber_completion",
    "subscriber_dead_letter"
  }

VARIABLES
  subscriberCompletions,     \* [subscriber |-> set of completion keys observed]
  subscriberAttempts,        \* [subscriber |-> [event -> attempt count]]
  subscriberDeadLetters      \* [subscriber |-> set of event ids]

Matched(s, subscriber, log) ==
  { i \in 1..Len(log) :
      /\ log[i].kind \in { "permission_requested", "prompt_turn_started", ... }  \* subscriber-specific filter
      /\ ... subscriber-filter-predicate(log[i], subscriber) }

CompletedEventIds(s, subscriber, log) ==
  { log[i].logicalId : i \in 1..Len(log) :
      /\ log[i].kind = "subscriber_completion"
      /\ log[i].subscriberName = subscriber }

PendingForSubscriber(s, subscriber, log) ==
  { log[i].logicalId : i \in Matched(s, subscriber, log) }
    \ CompletedEventIds(s, subscriber, log)

HandleMatchedEvent(s, subscriber, i, completionKey) ==
  \* Successful handle → append completion envelope.
  /\ i \in Matched(s, subscriber, sessionLog[s])
  /\ log[i].logicalId \notin CompletedEventIds(s, subscriber, sessionLog[s])
  /\ sessionLog' = [sessionLog EXCEPT ![s] = Append(@, CompletionEnvelope(subscriber, completionKey))]
  /\ subscriberCompletions' = [subscriberCompletions EXCEPT ![subscriber] = @ \cup {completionKey}]
  /\ ... invariants preserved

RetryMatchedEvent(s, subscriber, i) ==
  \* Transient failure → increment attempt count, don't append completion.
  /\ subscriberAttempts' = [subscriberAttempts EXCEPT ![subscriber][i] = @ + 1]
  /\ subscriberAttempts'[subscriber][i] <= MaxAttempts

DeadLetterMatchedEvent(s, subscriber, i) ==
  \* Attempts exhausted → write dead-letter envelope.
  /\ subscriberAttempts[subscriber][i] >= MaxAttempts
  /\ sessionLog' = [sessionLog EXCEPT ![s] = Append(@, DeadLetterEnvelope(subscriber, i))]
  /\ subscriberDeadLetters' = [subscriberDeadLetters EXCEPT ![subscriber] = @ \cup {log[i].logicalId}]
```

### 5.3 Proposed invariants

```tla
\* Every matched event eventually gets a completion or dead-letter.
SubscriberEventualCompletion ==
  \A s \in Sessions :
    \A subscriber \in SubscriberNames :
      \A id \in { log[i].logicalId : i \in Matched(s, subscriber, sessionLog[s]) } :
        \/ id \in CompletedEventIds(s, subscriber, sessionLog[s])
        \/ id \in subscriberDeadLetters[subscriber]
        \/ subscriberAttempts[subscriber][id] < MaxAttempts  \* still in retry

\* Completion envelopes are unique per (subscriber, event).
SubscriberCompletionUnique ==
  \A s \in Sessions :
    \A subscriber \in SubscriberNames :
      \A key \in CompletionKeys :
        Cardinality({ i \in 1..Len(sessionLog[s]) :
          /\ sessionLog[s][i].kind = "subscriber_completion"
          /\ sessionLog[s][i].subscriberName = subscriber
          /\ sessionLog[s][i].completionKey = key }) <= 1

\* Replay from any offset produces the same completion set.
SubscriberReplayIdempotent ==
  \A s \in Sessions :
    \A offset \in 0..Len(sessionLog[s]) :
      CompletedEventIds(s, subscriber, sessionLog[s]) =
        CompletedEventIds(s, subscriber, SubSeq(sessionLog[s], 1, offset))
          \cup { log[i].logicalId : i \in (offset+1)..Len(sessionLog[s])
                                   /\ sessionLog[s][i].kind = "subscriber_completion" }
  \* In plain English: the set of completions after replay equals the
  \* set of completions already in the prefix plus any new ones since.

\* Dead-letter implies MaxAttempts reached.
SubscriberDeadLetterGated ==
  \A subscriber \in SubscriberNames :
    \A id \in subscriberDeadLetters[subscriber] :
      subscriberAttempts[subscriber][id] >= MaxAttempts

\* Progress: Pending set is monotone non-increasing when no new matches arrive.
SubscriberProgressMonotone ==
  lastAction \in { "handle_matched_event", "dead_letter_matched_event" } =>
    \A s \in Sessions, subscriber \in SubscriberNames :
      PendingForSubscriber(s, subscriber, sessionLog[s]) \subseteq
        PendingForSubscriber(s, subscriber, previousSessionLog[s])
```

### 5.4 What this buys us

Stateright + TLC can exhaustively check these invariants over small configurations. The existing `verification/stateright/` harness already runs `managed_agents.tla` properties — adding these invariants extends the same infrastructure.

Model checking validates:
- No event is silently dropped
- Retries are bounded
- Replay is deterministic
- Dead-letter is reachable only through exhaustion

These are the correctness properties any real subscriber implementation must satisfy. Once the spec passes, any concrete `DurableSubscriber` impl that preserves the state transitions inherits the invariants.

---

## 6. Implementation plan

### Phase 1 — Extract the trait, re-express `ApprovalGateComponent` (no behavior change)

1. Define `DurableSubscriber` trait + `HandlerOutcome` + `RetryPolicy` in a new crate module `fireline-harness/src/subscriber/mod.rs`.
2. Implement `ApprovalGateSubscriber` that wraps the existing approval-gate logic.
3. Keep `ApprovalGateComponent` as the existing topology component, but refactor its internals to delegate to `DurableSubscriber`.
4. All existing tests pass unchanged — this is a pure refactor.

**Acceptance:** `cargo test --workspace` passes, approval-gate behavior is byte-identical on the stream.

### Phase 2 — Add the driver + webhook subscriber

1. Implement `DurableSubscriberDriver` as a new topology component (`"durable_subscriber"` registration).
2. Implement `WebhookSubscriber` as the first active subscriber.
3. Add TypeScript `webhook()` middleware helper, wire through `middlewareToComponents`.
4. Integration test: compose spec with `webhook()` middleware, provision, trigger matching events, assert webhook fires + `webhook_delivered` envelope appears on stream.

**Acceptance:** examples/webhook-integration/ demo runs end-to-end.

### Phase 3 — Additional subscribers

1. `AutoApproveSubscriber` (replaces inline auto-approval in application code).
2. `PeerCallSubscriber` (durable cross-agent dispatch — currently partial in `peer_mcp`).
3. `WakeTimerSubscriber` (scheduled wake — enables deferred work).
4. Update docs + examples for each.

### Phase 4 — TLA+ verification

1. Extend `verification/spec/managed_agents.tla` with subscriber state variables and actions.
2. Add the four invariants (`SubscriberEventualCompletion`, `SubscriberCompletionUnique`, `SubscriberReplayIdempotent`, `SubscriberProgressMonotone`).
3. Run TLC to exhaustively check small configurations.
4. Port to `verification/stateright/` for concurrent-correctness checks.

**Acceptance:** TLC finds no counterexamples for configurations covering at least 3 subscribers × 5 events × 3 replay offsets × retry-up-to-3-attempts.

### Phase 5 — Migration docs + example sweep

1. Update `docs/guide/approvals.md` to frame the approval gate as one instance of `DurableSubscriber`.
2. Update `examples/approval-workflow/` to use `webhook()` + `autoApprove()` instead of the inline subscriber pattern that can duplicate side effects on crash.
3. Deprecate any ad-hoc subscriber patterns in examples.

---

## 7. What this does NOT solve

Worth being explicit about the limits:

- **External system deduplication.** If your webhook receiver isn't idempotent and the subscriber fires twice due to a clean retry (not a crash), the receiver sees duplicates. Subscribers should include idempotency keys in outbound requests; receivers must dedupe. Fireline cannot enforce this.
- **Arbitrary handler logic.** A `handle()` that spawns threads, mutates global state, or writes to non-stream sinks breaks the replay model. The primitive is disciplined — handlers must follow the contract. Bad handlers make bad subscribers.
- **Cross-subscriber coordination.** Two subscribers both reacting to the same event independently is fine. Two subscribers that need to coordinate ("do X only if Y subscriber finished") is not a native pattern — compose through additional completion events on the stream.
- **Hot reload of subscriber config.** Changing a subscriber's filter or URL requires restarting the driver. Live-editing subscribers is future work (likely via a `subscriber_config_updated` envelope that the driver watches for).

---

## 8. Open questions

1. **Retry state storage.** Attempts and backoff schedules can live in-memory (lost on restart, which means the schedule restarts) or on the stream (persistent, but pollutes the log). Recommend on-stream for first cut with periodic compaction.
2. **Subscriber epoch semantics.** If a subscriber's config changes (new filter, new handler), do pending events from the old config still get handled? Proposal: yes, old completions are honored, new events use the new config. Key by `(subscriber_name, epoch)` if conflicts arise.
3. **Cross-stream subscribers.** Today every subscriber reads one state stream. For cross-agent operations, a subscriber might want to observe multiple streams. Start with single-stream; cross-stream can compose through a "forwarder" subscriber that copies events between streams.
4. **Dead-letter handling UX.** Who sees dead-lettered events? A subscriber watching `subscriber_dead_letter` kinds on the same stream (meta-subscriber). Or export to an external DLQ system. Recommend meta-subscriber for simplicity.
5. **Passive vs active detection.** Is the trait distinction explicit (`trait PassiveSubscriber` vs `trait ActiveSubscriber`) or an enum at construction time? Recommend enum (`SubscriberMode::{Active, Passive}`) to keep one trait.

---

## 9. References

- **Pattern origin:** `crates/fireline-harness/src/approval.rs` — the working prototype
- **Durable streams primitive:** [durablestreams.com/stream-db](https://durablestreams.com/stream-db) — `LiveMode::Sse` + offset replay
- **TLA+ spec:** `verification/spec/managed_agents.tla` — invariants to extend
- **Stateright harness:** `verification/stateright/src/lib.rs` — concurrent-correctness checks
- **Flamecast webhook RFC:** referenced as a concrete target shape for webhook delivery semantics
- **Related proposal:** `docs/proposals/webhook-support.md` — earlier sketch that this subsumes
