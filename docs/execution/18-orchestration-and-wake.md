# 18: Orchestration and the Wake Primitive

Status: planned
Type: execution slice

Related:

- [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md)
- [`../runtime/control-and-data-plane.md`](../runtime/control-and-data-plane.md)
- [`../runtime/heartbeat-and-registration.md`](../runtime/heartbeat-and-registration.md)
- [`./13-distributed-runtime-fabric/13c-first-remote-provider-docker-via-bollard.md`](./13-distributed-runtime-fabric/13c-first-remote-provider-docker-via-bollard.md)
- [`./14-runs-and-sessions-api.md`](./14-runs-and-sessions-api.md)
- [`./16-capability-profiles.md`](./16-capability-profiles.md)
- [`./17-out-of-band-approvals.md`](./17-out-of-band-approvals.md)

## Primitive Anchor

Primitive extended: `Orchestration`

Acceptance-bar items this slice closes:

- `wake(runtime_key, reason)` primitive on the control plane
- in-process scheduler that calls `RuntimeProvider::start()` against stored
  durable state when no live runtime exists for the key
- retry-on-failure semantics with exponential backoff
- runtime-side contract for "catch up to durable state on start"
- documented external triggers: webhook ingress, approval resolution, peer call
  delivery, timer wake-ups

This slice establishes the orchestration substrate. The first product-level
consumer of that substrate is the future slice `16` approval flow, currently
documented in [`17-out-of-band-approvals.md`](./17-out-of-band-approvals.md)
until the numbering cleanup happens.

Depends on:

- slice `13c` (`Sandbox`) for a real cold-start path against a non-local
  provider
- slice `14` (`Session`) for the canonical durable read schema the wake path
  restores from

Unblocks:

- the future slice `16` approval flow, currently documented in
  [`17-out-of-band-approvals.md`](./17-out-of-band-approvals.md)
- later webhook, queue, multiplayer, and peer-delivery orchestration flows

## Objective

Add Fireline's missing orchestration primitive:

- `wake(runtime_key, reason)`

This slice makes "advance the agent" a control-plane operation rather than an
accident of a tab holding an ACP session open.

The first cut should stay intentionally narrow:

- the scheduler lives in-process inside `fireline-control-plane`
- the wake target is a `runtime_key`, not a new product object
- the runtime restores from durable state that already exists
- retry behavior is owned by the scheduler
- external triggers all terminate at the same wake surface

## Product Pillar

Durable orchestration.

## User Workflow Unlocked

A consuming product can treat Fireline as durable background-agent substrate
instead of only as runtime hosting.

Examples:

- a webhook arrives while no browser tab is open and still advances work
- an approval is resolved later and resumes the blocked session
- a peer runtime delivers a call to a dormant target runtime
- a timer fires and wakes scheduled work

The important user is a control plane, workflow service, or product embedding
Fireline and needing one honest "continue this durable unit of work" primitive.

## Scope

### 1. Wake entry point

Add a wake surface owned by the control plane:

- in-process function: `wake(runtime_key, reason)`
- HTTP wrapper: `POST /v1/runtimes/{runtimeKey}/wake`

The request body should stay intentionally small. First-cut shape:

```ts
type WakeReason =
  | { kind: "manual" }
  | { kind: "approval_resolved"; approval_id: string }
  | { kind: "webhook"; source: string; event_id?: string }
  | { kind: "peer_delivery"; from_runtime_key: string; call_id?: string }
  | { kind: "timer"; timer_id: string };
```

This slice does not need a rich routing DSL. It needs a stable reason envelope
the scheduler can log, dedupe against, and expose in diagnostics.

### 2. In-process scheduler in `fireline-control-plane`

Implement the scheduler as an in-process service inside the control plane.

Responsibilities:

- accept wake requests from HTTP or internal callers
- coalesce duplicate wake requests for the same `runtime_key`
- decide whether a healthy live runtime already exists
- call `RuntimeProvider::start()` when no live runtime can satisfy the wake
- observe registration/readiness and retry bounded failures

The scheduler should be boring:

- one queue of pending wake requests
- one in-flight record per `runtime_key`
- one retry policy
- one status surface for operators and tests

It should not try to be a workflow engine. It is a delivery mechanism for the
single primitive "ensure this durable unit of work is able to make progress
now."

### 3. Runtime wake lifecycle

This slice should define the runtime-side lifecycle after a wake-triggered
start.

Required phases:

1. provider launches compute for the requested `runtime_key`
2. runtime registers with the control plane
3. runtime restores durable read state for that `runtime_key`
4. runtime catches up to the latest durable offset before processing new work
5. runtime declares itself ready for wake-driven work

The important rule is that registration is not enough. A newly started runtime
is not "awake" until it has replayed durable state far enough to safely resume.

### 4. Catch-up-to-durable-state contract

This slice depends on slice `14` because wake is only sound if the runtime can
restore from a stable durable read contract.

The contract for "catch up on start" should be explicit:

- the runtime identifies the durable stream and session lineage for its
  `runtime_key`
- runtime-local materializers replay from the last known durable offset or from
  the canonical session root when required
- the runtime rebuilds the minimal in-memory indexes needed to resume work
- conductor components that own durable wait state must have a defined restore
  hook
- no new external wake work is processed until catch-up completes

This slice does not require every possible component to implement restore
semantics. It does require the contract for where that restore logic lives and
when it runs.

### 5. Retry semantics

Retry behavior belongs to the scheduler, not to callers.

First-cut policy:

- callers submit a wake request and receive an acknowledgement or an immediate
  conflict/error
- the scheduler retries failed launch or readiness attempts with exponential
  backoff
- retries are keyed by `runtime_key`
- duplicate wake requests for the same `runtime_key` should attach to the same
  in-flight wake attempt rather than spawning parallel runtimes

The retry contract should answer:

- which errors are retryable and when the scheduler gives up
- how last failure and superseding wake requests are recorded

This slice should not overdesign dead-letter queues or multi-region failover.
It does need a deterministic local policy that tests can assert against.

### 6. External triggers

All external "continue work" events should converge on the same wake surface.

This slice should explicitly document four first-class trigger families:

- webhook ingress
- approval resolution
- peer call delivery
- timer wake-ups

#### Webhook ingress

An ingress component or HTTP endpoint receives an external event, persists the
relevant durable record, and then calls `wake(runtime_key, reason)`.

#### Approval resolution

The future slice `16` approval flow consumes the orchestration substrate
defined here.

The orchestration responsibility is:

- a durable wait record can point at a blocked `runtime_key`
- resolving that record can call `wake(runtime_key, { kind:
  "approval_resolved", ... })`

Human-facing approval queues and product APIs are out of scope here. The wake
contract is the substrate they rely on.

#### Peer call delivery

Cross-runtime calls should not require the callee to already be live.

The delivery shape is:

- persist a durable inbound peer-call record or delivery marker
- call `wake(target_runtime_key, { kind: "peer_delivery", ... })`
- let the resumed runtime read and handle that durable record

#### Timer wake-ups

A timer service, cron-like helper, or in-process scheduler can call
`wake(runtime_key, { kind: "timer", ... })`.

This slice does not need a final timer product. It needs the contract that
timer-driven wake is just another wake reason, not a special execution path.

### 7. Observable wake state and first proof path

This slice should make wake attempts inspectable enough for operators and tests
to reason about them.

At minimum the control plane should expose or retain:

- last requested wake reason
- wake attempt status: queued, starting, catching_up, ready, failed
- retry count
- last failure
- timestamps for queued/start/ready/failure transitions

This does not have to be a new end-user product API. It can be runtime
descriptor metadata, a scheduler status endpoint, or test-only introspection as
long as the implementation is observable.

The first end-to-end proof for this slice should be orchestration-only, not a
full approvals product flow.

Recommended proof:

- create durable session state for a runtime
- stop or delete live compute
- issue a wake request
- scheduler starts fresh compute through a real provider path
- runtime registers, catches up, and becomes ready
- the resumed runtime processes one wake-driven durable input

This keeps the slice honest without dragging product-level approval UX into the
same PR.

## Explicit Non-Goals

This slice does **not** require:

- a new Fireline-owned workflow engine
- a DAG scheduler
- a queue product with pause, reprioritize, or visibility-timeout semantics
- a human-facing approval system
- a final webhook catalog or timer catalog
- cross-region orchestration
- reworking ACP transport semantics
- inventing a second durable event model separate from the session stream
- broad component-resume support beyond defining the restore contract and
  proving the first viable path

It also does **not** replace the future slice `16` approval flow, currently
documented in [`17-out-of-band-approvals.md`](./17-out-of-band-approvals.md).
Slice `18` owns the orchestration primitive; the approval slice owns the first
durable wait/resume consumer built on top of it.

## Acceptance Criteria

- the control plane exposes a wake entry point by `runtime_key`
- `wake(runtime_key, reason)` is available both as an in-process scheduler call
  and as an HTTP endpoint
- the control plane owns an in-process scheduler that:
  - coalesces duplicate in-flight wakes per `runtime_key`
  - launches compute through `RuntimeProvider::start()` when needed
  - waits for registration and logical readiness
  - retries failures with exponential backoff
- the runtime-side wake contract is documented and implemented for the first
  path:
  - start with existing durable state
  - replay/catch up before resuming work
  - refuse new wake-driven work until catch-up completes
- wake reasons are explicitly represented for:
  - manual
  - approval resolution
  - webhook ingress
  - peer delivery
  - timer
- at least one orchestration-only end-to-end proof shows:
  - a dormant runtime can be woken
  - a fresh runtime instance can catch up to prior durable state
  - wake-driven work resumes after catch-up
- the implementation leaves a clear handoff seam for the future slice `16`
  approval flow to resolve a durable wait record and call `wake()` rather than
  inventing a second resume path

## Validation

- `cargo test -q`
- one control-plane integration test that:
  - creates durable state for a runtime without keeping live compute around
  - calls `POST /v1/runtimes/{key}/wake`
  - verifies duplicate wake requests coalesce
  - verifies the runtime does not report ready until catch-up completes
- one retry-path integration test that:
  - injects a launch or readiness failure
  - verifies exponential backoff retries occur
  - verifies a later successful attempt reaches ready
- one durable-catch-up integration test that:
  - writes durable session state
  - starts a fresh runtime instance
  - proves the runtime rebuilds the minimal read model before resuming work
- one trigger-surface test suite that verifies all documented trigger families
  normalize into the same scheduler wake path:
  - webhook ingress
  - approval resolution
  - peer delivery
  - timer

## Handoff Note

This is the first execution doc that should be written entirely from the
managed-agents primitive framing rather than from a product-surface framing.

The handoff should emphasize:

- this slice extends `Orchestration`, not Sessions, Tools, or approvals
- keep the scheduler in-process inside `fireline-control-plane`
- depend on slice `14` for canonical durable read semantics rather than
  inventing a new restore model here
- prove wake against a real cold-start provider path from slice `13c`
- do not let approval-specific UX or queue-product concerns leak into the
  primitive
