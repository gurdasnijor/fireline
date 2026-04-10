# 17: Out-of-Band Approvals

Status: planned
Type: execution slice

Related:

- [`../product/out-of-band-approvals.md`](../product/out-of-band-approvals.md)
- [`../product/runs-and-sessions.md`](../product/runs-and-sessions.md)
- [`../product/product-api-surfaces.md`](../product/product-api-surfaces.md)
- [`../product/priorities.md`](../product/priorities.md)
- [`../product/roadmap-alignment.md`](../product/roadmap-alignment.md)
- [`../state/session-load.md`](../state/session-load.md)
- [`./14-runs-and-sessions-api.md`](./14-runs-and-sessions-api.md)
- [`./16-capability-profiles.md`](./16-capability-profiles.md)

## Objective

Prove the first durable `ApprovalRequest` and run wait-state model so a Fireline
run can:

- pause on a gated action
- persist that wait durably
- be serviced later by a browser, control-plane UI, or operator path
- resume or terminate without depending on the original interactive client

This slice should keep the first implementation intentionally small:

- one durable approval-request record type
- one waiting run state
- approve / deny / expire
- one service path

## Product Pillar

Reusable conductor extensions.

## User Workflow Unlocked

A long-running run can:

- hit a gated action
- transition into a durable waiting state
- show up in a later browser or operator queue
- continue only after a human or external service responds

This is the first real proof that Fireline sessions can survive beyond a single
foreground interactive client.

## Why This Slice Exists

Without durable approvals, Fireline remains strongest only in foreground,
interactive flows.

With durable approvals, Fireline becomes much more compelling for:

- long-running background coding runs
- risky-action approvals
- credential-connect flows
- later operator intervention

That is a major part of the durable-agent-fabric story.

## Scope

### 1. ApprovalRequest product object

Define a first-cut `ApprovalRequest` product object that sits above lower-level
permission or pending-request rows.

Required first-cut fields:

- `requestId`
- `runId`
- `sessionId?`
- `promptTurnId?`
- `kind`
- `state`
- `title`
- `description?`
- `requestedCapabilities?`
- `options?`
- `createdAtMs`
- `resolvedAtMs?`
- `expiresAtMs?`

Required first-cut request kinds:

- `permission`
- `credential_connect`
- `operator_approval`
- `policy_escalation`

Required first-cut states:

- `pending`
- `approved`
- `denied`
- `expired`
- `cancelled`
- `orphaned`

### 2. Run wait-state projection

Define the run-level waiting view explicitly.

At minimum, a run in `waiting` should answer:

- which request ids are blocking it
- what kind of wait is active
- since when it has been waiting
- whether the wait is resumable

This wait-state must be durable and externally inspectable.

### 3. Product API surface

Add first-cut product-layer approval APIs:

```ts
client.approvals.list(filter?)
client.approvals.get(requestId)
client.approvals.approve(requestId, payload?)
client.approvals.deny(requestId, reason?)
client.approvals.expire(requestId)
```

And at the run layer:

```ts
client.runs.get(runId)
client.runs.list({ state: "waiting" })
```

### 4. One gate path from conductor to durable request

This slice should include one real gate path from conductor behavior to durable
approval state.

Examples:

- risky permission gate
- credential-connect gate
- policy-escalation gate

Only one is required for the first cut. The point is to prove the end-to-end
model, not the whole gate matrix.

### 5. Resume semantics after service

This slice must make explicit:

- what resumes blocked work
- what terminates it
- how resume works when the original client transport is gone

The resume path must be owned by durable runtime/session substrate or the
control plane, not by a one-off browser tab.

### 6. One service path

One real surface must be able to service requests.

Examples:

- browser harness
- control-plane UI
- operator-facing web surface

The proof needed is durability and serviceability, not broad UI coverage.

## Explicit Non-Goals

This slice does **not** require:

- a rich workflow engine
- complex multi-step approval chains
- broad policy DSL design
- fully solving shared-session or multiplayer semantics
- every possible wait type
- making approval service depend on an always-live interactive client

## Acceptance Criteria

- `ApprovalRequest` exists as an explicit durable product object
- a run can enter a first-cut `waiting` state with blocking request ids
- `client.approvals.list/get/approve/deny/expire` exist
- one conductor gate path can create a durable request and block the run
- approve / deny / expire all produce deterministic durable outcomes
- blocked work can resume or terminate after later service without requiring
  the original client connection to still exist
- one browser or control-plane service path can inspect and resolve requests

## Validation

- `cargo test -q`
- `pnpm --filter @fireline/client test`
- one integration test that:
  - starts a run that hits one gated action
  - creates a durable approval request
  - observes the run enter `waiting`
  - services the request later through the product API
  - verifies the run resumes or terminates correctly
- one consumer-oriented integration test that:
  - lists waiting runs
  - fetches approval detail
  - resolves the request through the service surface

## Handoff Note

Keep the first cut deliberately small.

Do not:

- turn this into a general workflow engine
- solve every approval pattern at once
- make the product model depend on shared-session bridge work
- overfit the first schema to one UI

The key proof is:

- Fireline can pause durably on a gated action
- the wait becomes an explicit serviceable object
- later service can resume the work safely

