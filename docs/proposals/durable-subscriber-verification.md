# Durable Subscriber Verification

> Status: design
> Date: 2026-04-12
> Scope: verification additions for `docs/proposals/durable-subscriber.md` and the passive/imperative projection in `docs/proposals/durable-promises.md`

This document specifies the verification work needed to make regressions in the DurableSubscriber substrate mechanically detectable.

It is intentionally redundant across layers:

- TLA+ proves the subscriber state machine, completion-key semantics, replay behavior, and trace-context rules.
- Stateright checks race, replay, timeout, and rebuild behavior against a refinement of the live runtime.
- Mechanical audits fail the build when code drifts back to ad hoc completion keys, direct completion writes, or missing trace propagation.
- Migration fixtures prove the approval-gate to DurableSubscriber refactor preserves the already-proven substrate behavior.
- End-to-end tests prove approval, webhook delivery, peer routing, wake timers, auto-approve, and awakeable sugar all respect the same invariant set against the live stack.

This document is the binding Phase 0 gate from [durable-subscriber-execution.md](./durable-subscriber-execution.md): no implementation phase after the docs-only prerequisite may land until this invariant register exists on `main`.

The design is intentionally about what must be verified, not how the Rust trait surface is coded.

## Invariant Register

These invariant IDs are the stable proof targets later execution phases must cite.

| ID | Invariant | Meaning | Enforced by |
|---|---|---|---|
| `DSV-00 VerificationPlanExists` | Phase-0 binding gate | This document exists before implementation and later phases cite invariant IDs from it instead of inventing new wording ad hoc. | Docs gate, architect review |
| `DSV-01 CompletionKeyUnique` | Same-key completion uniqueness | Two passive subscribers or awakeables keyed identically resolve together exactly once; duplicate completion appends are semantic no-ops. | TLA+, Stateright, fixtures, E2E |
| `DSV-02 ReplayIdempotent` | Live path equals replay path | Replaying a stream containing a passive wait plus its resolution yields the same final state as the live path. | TLA+, Stateright, fixtures, E2E |
| `DSV-03 RetryBounded` | Active retries terminate | Active subscribers retry at most `N` times, then transition to dead-letter; no infinite retry loop is representable. | TLA+, Stateright, audits, E2E |
| `DSV-04 DeadLetterTerminal` | Dead-letter is terminal | Once a subscriber instance is dead-lettered, no further delivery attempts, completions, or cursor advances occur for that registration. | TLA+, Stateright, audits, E2E |
| `DSV-05 TraceContextPropagated` | Trace continuity across side effects | Every outbound side effect carries source `_meta.traceparent`; `tracestate` and `baggage` follow when present; completion envelopes preserve the same context. | TLA+, Stateright, audits, E2E |
| `DSV-10 SuspendResumeDurable` | Crash-safe passive wait | A blocked passive subscriber or awakeable survives runtime death and resumes from replay rather than minting a new logical wait. | Stateright, fixtures, E2E |
| `DSV-11 TimeoutObservableAndBounded` | Timeout is explicit and finite | Timeout produces a visible terminal outcome with no phantom completion and no indefinite half-wait state. | TLA+, Stateright, fixtures, E2E |
| `DSV-12 ConcurrentResolutionIsolatedByKey` | Key-scoped isolation | Resolving completion key A never releases waiters on key B, even under interleaving or replay. | TLA+, Stateright, fixtures, E2E |
| `DSV-13 RebuildRaceSafe` | Replay/live convergence | If replay rebuild and live completion append race, the resumed state converges to one logical completion without duplicate registration or duplicate emission. | Stateright, fixtures, E2E |

`DSV-10` through `DSV-13` are the carried-forward substrate obligations from [approval-gate-correctness.md](../reviews/approval-gate-correctness.md). DurableSubscriber does not get to weaken them; it generalizes them.

Awakeables are not a second substrate. Prompt/tool awakeables must satisfy `DSV-01` through `DSV-05` exactly as passive subscribers do, and any future step-scoped awakeable key must satisfy the same invariants once that key variant exists.

## Part 1: TLA+ Extensions

Target files: `verification/spec/managed_agents.tla`, `verification/spec/ManagedAgents.cfg`, and new `verification/spec/ManagedAgentsDurableSubscriber.cfg` or a sibling `verification/spec/durable_subscriber.tla` if the added state machine becomes too large for the current module.

### 1.1 Required model edits

Apply these structural edits before adding new invariants:

1. Add a DurableSubscriber state machine with terminal and non-terminal states:
   - `Registered`
   - `Waiting`
   - `Completed`
   - `TimedOut`
   - `DeadLettered`
2. Model passive registration and imperative awakeable suspension as the same logical operation:
   - source event matched
   - canonical `CompletionKey` derived
   - waiter enters `Registered`
   - passive path transitions to `Waiting`
   - matching completion transitions to `Completed`
3. Model active subscriber delivery separately from passive waiting:
   - delivery attempt count
   - retry budget
   - dead-letter transition
   - all bookkeeping remains infrastructure-plane state
4. Add a first-class completion log keyed by canonical `CompletionKey`, not by minted ids, correlation hashes, or subscriber-private identifiers.
5. Add a duplicate-resolution action whose only legal effect is no semantic state change for an already-completed key.
6. Add a replay action that reconstructs subscriber and awakeable state from the log at arbitrary offsets and must converge with the live path.
7. Add a trace-context carrier for outbound side effects and completion envelopes:
   - `traceparent`
   - `tracestate`
   - `baggage`
8. Model timeouts as explicit terminal outcomes. A timeout may race with a resolution, but the model must never admit a partially-completed state where both win.
9. Reserve an abstract step-scoped awakeable key behind a config flag so the imperative surface can be checked against the same invariants without introducing a separate substrate. Until that flag is enabled, the model only needs prompt/tool keys.

### 1.2 Helper definitions

Add helper carriers and derived sets for:

- `SubscriberNames`
- `CompletionKeys`
- `SubscriberModes == {Passive, Active}`
- `SubscriberStates == {Registered, Waiting, Completed, TimedOut, DeadLettered}`
- `AttemptsByRegistration`
- `DeadLetters`
- `CompletionsByKey`
- `TraceContextByKey`
- `ReplaySnapshots`

The model should be able to express:

- multiple registrations waiting on the same `CompletionKey`
- zero or one accepted completion winner per key
- duplicate completion attempts after a winner already exists
- active retry count and the configured retry bound
- passive suspend/resume for both subscriber registrations and awakeables

Required helper predicates:

- `IsTerminal(state)` for `Completed`, `TimedOut`, and `DeadLettered`
- `AcceptedCompletion(key)` for the first completion that wins for a key
- `DuplicateCompletionIsNoOp(key)` for later completion attempts
- `SameOutcomeAfterReplay(snapshot_a, snapshot_b)` for live-vs-replay equivalence
- `CarriesTraceContext(outbound, source)` for side-effect/completion trace propagation

### 1.3 Named invariants

#### `DSV-01 CompletionKeyUnique`

What it requires:

- a canonical `CompletionKey` has at most one semantic winner
- all still-waiting passive registrations bound to that key observe the same terminal completion
- later attempts to resolve the same key do not create a second completion outcome

What it forbids:

- duplicate semantic completions for one key
- one same-key waiter completing while another same-key waiter remains blocked
- active and passive consumers inventing separate completion identities for the same logical wait

#### `DSV-02 ReplayIdempotent`

What it requires:

- replaying a stream containing a passive wait plus its resolution yields the same final registration state as the live path
- replaying after a duplicate completion attempt yields the same result as replaying after only the first winner
- imperative awakeable resume is a refinement of passive subscriber replay, not a separate behavior

What it forbids:

- replay minting a fresh logical wait
- replay consuming a later duplicate completion differently than the live path
- live and replay disagreeing about whether the key is `Waiting`, `Completed`, `TimedOut`, or `DeadLettered`

#### `DSV-03 RetryBounded`

What it requires:

- every active subscriber registration carries a finite retry budget
- each retry consumes one unit of that budget
- after the final allowed retry, the only legal next state is `DeadLettered`

What it forbids:

- unbounded retry loops
- retry counters resetting on replay
- success/completion being recorded after the retry budget is exhausted unless the winning completion was already durable before dead-letter transition

#### `DSV-04 DeadLetterTerminal`

What it requires:

- once a registration is `DeadLettered`, it stays terminal
- no more outbound attempts fire
- no cursor or progress state advances for that registration

What it forbids:

- retry-after-dead-letter
- late transition from `DeadLettered` back to `Waiting`
- completion-envelope append after dead-letter unless the completion already won before dead-lettering

#### `DSV-05 TraceContextPropagated`

What it requires:

- every outbound side effect copies source `_meta.traceparent`
- `tracestate` and `baggage` follow when present
- every completion envelope emitted by an active subscriber carries the same trace context as the triggering event
- passive completions appended by external writers join the same trace lineage rather than inventing subscriber-local correlation fields

What it forbids:

- outbound HTTP, ACP, or peer traffic without W3C trace context
- completion envelopes that lose trace continuity from the source event
- subscriber-specific lineage ids replacing `_meta` propagation

#### Carried-forward approval obligations

The model must also explicitly encode these inherited invariants:

- `DSV-10 SuspendResumeDurable`
- `DSV-11 TimeoutObservableAndBounded`
- `DSV-12 ConcurrentResolutionIsolatedByKey`
- `DSV-13 RebuildRaceSafe`

These come directly from the already-proven approval substrate and are the semantic regression bar for the generalization.

### 1.4 Existing invariant updates

Update the existing verification surface so subscriber behavior composes with the canonical-id work instead of bypassing it:

- generalize any approval-specific "released only by matching approval" invariant into "released only by matching completion key"
- extend `_meta` propagation invariants so they cover subscriber-delivered HTTP/ACP side effects and subscriber-written completion envelopes, not just direct agent traffic
- keep plane-separation invariants intact by proving retry and dead-letter bookkeeping never appears on agent-plane rows
- ensure the TLA config used by DurableSubscriber runs only after the canonical-id baseline invariants are already green

## Part 2: Stateright Bindings

Target files: new `verification/stateright/src/durable_subscriber.rs`, update `verification/stateright/src/lib.rs`, update `verification/docs/refinement-matrix.md`.

### 2.1 Refinement mapping

The Stateright layer should add an abstract DurableSubscriber model and normalize live transitions into it.

Mapping table:

| Live transition | Source | TLA+ action | Stateright action |
|---|---|---|---|
| passive subscriber matches a source event | `durable-subscriber.md §5.1` and approval behavior in `approval.rs` | `RegisterPassiveWait` | `RegisterPassive { key }` |
| awakeable is declared | `durable-promises.md §1-3` | `RegisterPassiveWait` | `RegisterAwakeable { key }` |
| active subscriber dispatches an outbound effect | `durable-subscriber.md §5.2-5.6` | `DispatchActiveEffect` | `DispatchActive { subscriber, key }` |
| matching completion is appended | approval resolver, active subscriber completion, or external writer | `AppendCompletion` | `AppendCompletion { key }` |
| duplicate completion arrives | any duplicate writer path | `AppendDuplicateCompletion` | `AppendDuplicateCompletion { key }` |
| timeout fires | approval timeout or timer boundary | `TimeoutKey` | `TimeoutKey { key }` |
| retryable delivery failure occurs | active subscriber path | `ScheduleRetry` | `RetryDelivery { subscriber, key }` |
| retry budget exhausted | active subscriber path | `DeadLetterKey` | `DeadLetter { subscriber, key }` |
| replay/rebuild resumes from stream | approval rebuild path and durable-promises replay semantics | `ReplayFromOffset` | `ReplayFromOffset { offset }` |

Every modeled Stateright action must be derivable from an observed runtime transition or an explicit subscriber/awakeable semantic in the design docs.

### 2.2 New properties

Add these properties and register them in the Stateright test matrix:

1. `FirstResolutionWins`
   - concurrent resolution of the same key
   - first durable winner defines the terminal state
   - later resolutions are no-ops
   - proves `DSV-01`

2. `RebuildRaceConverges`
   - register a passive wait while its completion is being appended
   - replay/live interleaving must converge to one completed state
   - proves `DSV-02`, `DSV-10`, and `DSV-13`

3. `TimeoutAndResolutionAreAtomic`
   - timeout races with completion append
   - final state is exactly one of `Completed` or `TimedOut`
   - never partial, duplicated, or stuck
   - proves `DSV-11`

4. `RetryBudgetTerminates`
   - active delivery keeps failing
   - attempts stop at `N`
   - final state is `DeadLettered`
   - proves `DSV-03` and `DSV-04`

5. `TraceContextSurvivesDelivery`
   - outbound dispatch and completion append preserve source trace context across interleavings
   - proves `DSV-05`

6. `AwakeableReplayMatchesPassiveSubscriber`
   - the same key resolved through the imperative surface converges to the same model-level outcome as the passive subscriber path
   - proves `DSV-02` for `durable-promises.md`

Recommended test names:

- `durable_subscriber_model_first_resolution_wins`
- `durable_subscriber_model_rebuild_race_converges`
- `durable_subscriber_model_timeout_and_resolution_are_atomic`
- `durable_subscriber_model_retry_budget_terminates`
- `durable_subscriber_model_trace_context_survives_delivery`
- `durable_promises_model_matches_passive_subscriber_replay`

### 2.3 Counterexample shrinking

Use BFS and keep the alphabet intentionally small:

- `RegisterPassive`
- `RegisterAwakeable`
- `DispatchActive`
- `AppendCompletion`
- `AppendDuplicateCompletion`
- `RetryDelivery`
- `TimeoutKey`
- `DeadLetter`
- `ReplayFromOffset`

Shrink traces as tuples:

`(subscriber_or_surface, completion_key, action, prior_state, next_state)`

Expected minimal failures:

- 3 to 5 steps for same-key first-wins violations
- 4 to 6 steps for rebuild-race divergence
- 3 to 5 steps for timeout/resolution split-brain
- 2 to 4 steps for dead-letter non-terminality

## Part 3: Mechanical Audit Tooling

Target area: extend the existing `verification/audit` tooling and proposal-doc validation jobs so subscriber and awakeable regressions fail CI early.

These audits are phase-gated. They may be introduced before the refactor lands, but they only become required green gates once the canonical-id dependency is satisfied and the corresponding implementation phase begins.

### 3.1 Completion-key provenance audit

The build must fail if agent-layer or subscriber code mints completion identity outside the canonical `CompletionKey` surface.

Required audit behavior:

- scan subscriber-core, approval, peer, webhook, timer, and awakeable-facing codepaths
- fail on hashes, counters, UUIDs, prompt fingerprints, or ad hoc string concatenation used as semantic completion keys
- fail on any new agent-facing token such as `completion_id`, `delivery_id`, `subscriber_request_id`, or `awakeable_id` unless it is explicitly infra-only and annotated as such
- fail if prompt/tool awakeable resolution introduces a parallel key type instead of reusing `CompletionKey`

This audit enforces `DSV-01` and keeps `durable-promises.md` as sugar rather than a second identity surface.

### 3.2 Completion-envelope write-boundary audit

The build must fail if domain completion envelopes are appended from arbitrary codepaths instead of going through the subscriber substrate or the explicitly-allowed migration adapter.

Required audit behavior:

- scan for direct writes of completion kinds such as `approval_resolved`, `webhook_delivered`, `peer_call_delivered`, and `timer_fired`
- require those writes to originate from the approved completion writer boundary for the phase
- allow a temporary compatibility path only when the file carries an explicit migration annotation
- fail if the imperative awakeable resolver writes a parallel completion envelope shape instead of the substrate-owned completion shape

This audit enforces `DSV-01`, `DSV-02`, and the behavior-preserving migration bar.

### 3.3 Trace-propagation audit

The build must fail if outbound subscriber side effects bypass `_meta` trace propagation.

Required audit behavior:

- scan outbound HTTP, ACP, peer, timer callback, and integration delivery codepaths
- fail if source `_meta.traceparent` is not read and re-emitted
- fail if `tracestate` and `baggage` are dropped when present
- fail if completion envelopes emitted by active subscribers do not preserve the same trace context
- fail if subscriber code reintroduces Fireline-specific lineage fields instead of `_meta` propagation

This audit is the mechanical gate for `DSV-05`.

## Part 4: Migration Fixtures

Target files: new `tests/durable_subscriber_migration.rs` and `tests/fixtures/durable_subscriber/`.

The fixture goal is behavior preservation for the approval-gate to DurableSubscriber refactor, not byte-for-byte preservation of transitional identifiers. Compare normalized semantics:

- matched source event kind
- canonical completion key
- terminal outcome
- number of logical completion wins
- preserved trace context
- final agent-visible result

### 4.1 Reference fixture families

Add fixture families for the approval path already reviewed as correct:

- `approval-happy-path.ndjson`
- `approval-timeout.ndjson`
- `approval-concurrent-isolation.ndjson`
- `approval-rebuild-race.ndjson`
- `approval-duplicate-resolution.ndjson`

These fixtures should be produced from the current approval substrate before the refactor, then replayed against the refactored substrate after normalization to canonical completion-key semantics.

### 4.2 Required migration tests

1. `approval_refactor_preserves_suspend_resume_semantics`
   - proves `DSV-10`
   - one logical wait, one logical completion, resumed work continues exactly once

2. `approval_refactor_preserves_timeout_without_phantom_completion`
   - proves `DSV-11`
   - timeout yields terminal timeout/error state and no synthetic completion appears

3. `approval_refactor_preserves_concurrent_key_isolation`
   - proves `DSV-12`
   - resolving key A never releases key B

4. `approval_refactor_preserves_rebuild_race_convergence`
   - proves `DSV-13`
   - replay and live completion append converge without duplicate registration or duplicate completion

5. `approval_refactor_duplicate_resolution_is_no_op`
   - proves `DSV-01` and `DSV-02`
   - second resolution append changes no semantic outcome

### 4.3 Migration pass/fail rule

The refactor is behavior-preserving only if the old and new substrates agree on the normalized semantic trace for every reference fixture. Any divergence in logical completion count, terminal state, or trace-context continuity blocks the refactor.

## Part 5: End-to-End Scenarios

Target file: new `tests/durable_subscriber_e2e.rs` plus any shared harness helpers needed for subscriber profiles.

Every passive scenario should eventually run through two surfaces:

- direct subscriber behavior
- the imperative awakeable surface from `durable-promises.md`

The invariant bar is the same for both.

### 5.1 Approval: passive wait, external resolution, replay

Test name:

- `durable_subscriber_approval_resolves_once_and_replays_cleanly`

Observable evidence:

- one logical `PromptKey(SessionId, RequestId)` wait is registered
- `approval_resolved` releases the blocked work exactly once
- replay yields the same final outcome
- duplicate resolution append is a no-op

Invariant coverage:

- `DSV-01`
- `DSV-02`
- `DSV-10`
- `DSV-11`
- `DSV-12`

### 5.2 Webhook delivery: retry, completion, dead letter

Test name:

- `durable_subscriber_webhook_delivery_is_bounded_and_trace_preserving`

Observable evidence:

- outbound delivery carries W3C trace headers derived from `_meta`
- completion is written back on success with the same trace context
- repeated failures stop at the configured retry bound
- after dead-letter, no more attempts occur

Invariant coverage:

- `DSV-03`
- `DSV-04`
- `DSV-05`

### 5.3 Peer routing: caller-local key, first-wins completion

Test name:

- `durable_subscriber_peer_routing_preserves_key_and_trace_context`

Observable evidence:

- caller-side completion key remains caller-local and canonical
- outbound and inbound peer envelopes carry the same trace lineage through `_meta`
- duplicate peer-delivery acknowledgments do not create a second semantic completion

Invariant coverage:

- `DSV-01`
- `DSV-02`
- `DSV-05`

### 5.4 Wake timer: timeout/resolution race

Test name:

- `durable_subscriber_wake_timer_race_converges_without_split_brain`

Observable evidence:

- timer fire and external resolution can race
- only one terminal outcome wins
- replay reconstructs the same winner

Invariant coverage:

- `DSV-02`
- `DSV-11`

### 5.5 Auto-approve: active and passive approval paths interoperate

Test name:

- `durable_subscriber_auto_approve_shares_the_same_completion_spine`

Observable evidence:

- auto-approve writes the same logical completion key as the passive approval path
- no second approval identity exists
- passive/manual and active/auto paths can coexist without ambiguity

Invariant coverage:

- `DSV-01`
- `DSV-02`
- `DSV-05`

### 5.6 Awakeable parity: imperative surface is only sugar

Test name:

- `durable_promises_awakeable_matches_passive_subscriber_semantics`

Observable evidence:

- `ctx.awakeable(...).promise` suspends on the same completion key a passive subscriber would wait on
- `resolveAwakeable(...)` is indistinguishable from appending the same matching completion envelope
- replay after an awakeable resolution converges to the same final state as the direct subscriber path

Invariant coverage:

- `DSV-01`
- `DSV-02`
- `DSV-05`

Post-test rule:

- the Part 3 audits must run in the same PR as the E2E tests
- a green happy path without the audit suite does not count as acceptance

## Appendix: Phase Gates

These are the minimum verification gates later execution phases must cite.

| Execution phase | Required gate |
|---|---|
| Phase 0: verification doc prerequisite | `DSV-00` present; architect review checklist below is complete |
| Phase 1: substrate extraction | TLA+ checks for `DSV-01` through `DSV-05` exist; audit scaffolding exists and can fail on a seeded regression |
| Phase 2: approval refactor | migration fixtures for `DSV-10` through `DSV-13` pass; Stateright replay/race properties are green |
| Phase 3: webhook delivery | `DSV-03`, `DSV-04`, and `DSV-05` are green in Stateright, audit, and E2E layers |
| Phase 4: auto-approve | same completion-spine and coexistence checks are green in fixture and E2E layers |
| Phase 5: peer + timer subscribers | peer trace propagation and timer race convergence are green in Stateright and E2E layers |
| Phase 6+: durable-promises surface | awakeable parity checks are green; no parallel awakeable-id surface appears in audits |

## Architect Review Checklist

- [ ] The document mirrors the five-layer verification structure used by `acp-canonical-identifiers-verification.md`.
- [ ] `DSV-01 CompletionKeyUnique`, `DSV-02 ReplayIdempotent`, `DSV-03 RetryBounded`, `DSV-04 DeadLetterTerminal`, and `DSV-05 TraceContextPropagated` are all defined and mapped to verification layers.
- [ ] The already-proven approval substrate obligations are carried forward as `DSV-10` through `DSV-13`, not dropped as "approval-only" quirks.
- [ ] Awakeables are treated as the imperative projection of passive subscribers, not as a second workflow substrate with independent ids or completion semantics.
- [ ] No verification layer assumes Fireline-minted completion ids, lineage rows, or infra-plane leakage into agent-plane completions.
- [ ] Trace propagation requirements cover both outbound side effects and completion envelopes.
- [ ] The migration-fixture section proves behavior preservation rather than byte-for-byte preservation of transitional fields.
- [ ] The Phase-0 gate is explicit: later execution phases can cite stable invariant IDs from this document before code lands.

## References

- [durable-subscriber.md](./durable-subscriber.md)
- [durable-promises.md](./durable-promises.md)
- [acp-canonical-identifiers-verification.md](./acp-canonical-identifiers-verification.md)
- [durable-subscriber-execution.md](./durable-subscriber-execution.md)
- [approval-gate-correctness.md](../reviews/approval-gate-correctness.md)
- [approval.rs](../../crates/fireline-harness/src/approval.rs)
