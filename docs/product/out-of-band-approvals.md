# Out-of-Band Approvals

> Related:
> - [`index.md`](./index.md)
> - [`product-api-surfaces.md`](./product-api-surfaces.md)
> - [`ecosystem-story.md`](./ecosystem-story.md)
> - [`priorities.md`](./priorities.md)
> - [`../programmable-topology-exploration.md`](../programmable-topology-exploration.md)
> - [`../state/session-load.md`](../state/session-load.md)
> - [`../ts/primitives.md`](../ts/primitives.md)
> - [`../execution/10-acp-shared-session-bridge.md`](../execution/10-acp-shared-session-bridge.md)

## Purpose

One of Fireline's strongest product stories is the ability to let a long-running
run pause on a gated action and continue only after a human or external service
responds later.

This doc defines that behavior at the product layer.

The important shift is:

- the run should appear to "block"
- but the wait itself must be durable, externally serviceable, and restart-safe

## Why This Matters

Without out-of-band approvals, Fireline remains strongest only in interactive,
foreground flows.

With out-of-band approvals, Fireline can power:

- long-running background coding agents
- delayed approvals for risky actions
- OAuth or credential connect flows that need a human later
- agents that continue even after the original interactive client has
  disconnected

This is a major part of the "session outside the harness" story.

## What Counts As An Approval

This doc uses "approval" broadly.

It includes:

- a direct allow/deny decision
- a permission prompt
- a credential connect request
- a request for higher policy scope
- a request that must be serviced by an operator or external system

So the durable object is better thought of as a **serviceable wait request**.

## Core Product Behavior

When a run reaches a gated action:

1. a conductor component intercepts the action
2. Fireline creates a durable approval/request record
3. the run transitions into a waiting state
4. a browser, mobile surface, Slackbot, or operator UI can service the request
5. Fireline resumes or terminates the run based on the decision

The key rule is:

**the decision path must survive disconnects and process restarts.**

## Product Objects

At the product layer there should be two related objects:

### ApprovalRequest

The unit of waiting.

Suggested questions it should answer:

- what run/session is blocked?
- why is it blocked?
- what needs to be decided or supplied?
- who can service it?
- what happens if it expires?

### RunWaitState

The run-level view of blocked execution.

Suggested questions it should answer:

- is the run currently waiting?
- how many pending requests exist?
- what is the blocking reason?
- can the run still be resumed?

## Strawman Approval Shape

```ts
type ApprovalRequest = {
  requestId: string
  runId: string
  sessionId?: string
  promptTurnId?: string

  kind:
    | "permission"
    | "credential_connect"
    | "operator_approval"
    | "policy_escalation"

  state: "pending" | "approved" | "denied" | "expired" | "cancelled" | "orphaned"

  title: string
  description?: string
  requestedCapabilities?: string[]
  options?: ApprovalOption[]

  createdAtMs: number
  resolvedAtMs?: number
  expiresAtMs?: number
}
```

## Mapping To What Fireline Already Has

Fireline already has some relevant durable state shapes:

- `pending_request`
- `permission`

in [packages/state/src/schema.ts](/Users/gnijor/gurdasnijor/fireline/packages/state/src/schema.ts).

That is a strong starting point, but the product story likely needs a cleaner
surface above those lower-level rows.

In other words:

- current state rows are the substrate
- `ApprovalRequest` is the product object

## Relationship To ACP `session/request_permission`

ACP already has a permission mechanism.

That is useful, but not sufficient by itself for Fireline's product goal.

Why:

- ACP request/response alone does not define durable waiting
- it does not define how a disconnected or absent human services the request
- it does not define control-plane views, expiration, or later resume

So Fireline should use ACP permission flows where appropriate, but layer a
durable product model on top.

## Types Of Out-of-Band Service

### 1. Interactive approval

Examples:

- approve this deploy
- allow this shell action
- let this agent call a sensitive tool

### 2. Credential connect

Examples:

- user needs to complete OAuth in a browser
- user needs to attach a GitHub or Slack connection
- a vault path needs human confirmation

This is where the `agent.pw` story fits naturally.

### 3. Operator decision

Examples:

- an on-call engineer approves escalation
- an admin grants temporary policy scope
- a reviewer decides whether a subagent can proceed

## Run States

At the product layer, a run should be able to represent waiting clearly.

At minimum:

- `running`
- `waiting`
- `completed`
- `failed`
- `cancelled`

`waiting` should include metadata such as:

- blocking request ids
- wait kind
- since when
- whether the wait is resumable

## Product Surface

This is where `client.approvals` becomes meaningful.

Suggested surface:

```ts
client.approvals.list(...)
client.approvals.get(requestId)
client.approvals.approve(requestId, payload?)
client.approvals.deny(requestId, reason?)
client.approvals.expire(requestId)
```

And at the run level:

```ts
client.runs.get(runId)
client.runs.list({ state: "waiting" })
```

## What Actually Resumes The Run

The product model should not depend on the original client transport still being
connected.

That means resume should be owned by:

- the runtime/session substrate
- or the control plane coordinating the runtime

not by a one-off browser tab.

This is why durable waiting is strongly tied to:

- session durability
- runtime-owned session state
- later control-plane-backed orchestration

## Relationship To Conductor Components

The first gate point will likely be conductor components.

Examples:

- approval gate for risky prompts or tools
- budget gate that requests override approval
- credential gate that pauses until a connection is established

These components should:

- detect the gated condition
- create durable request records
- surface the waiting state cleanly
- resume or fail deterministically after service

## First-Cut Recommendation

Keep the first implementation small.

Support:

- one durable approval-request record type
- one waiting run state
- approve / deny / expire
- one browser or control-plane service path

Defer:

- very rich policy DSLs
- complex multi-step approval chains
- concurrent attachment semantics
- broad workflow-engine features

## Questions The Next Slice Should Answer

1. What exact durable record shape should back `ApprovalRequest`?
2. How does a conductor component create and await that record?
3. What run/session state changes when waiting begins?
4. How does approval service resume the blocked work safely?
5. How do credential-connect waits differ from plain allow/deny waits?

## Non-Goals

This doc does not require:

- fully solving shared-session/multiplayer semantics
- solving every possible workflow wait pattern
- making approval service depend on an always-live interactive client

The goal is narrower:

**a Fireline run should be able to pause durably on a gated action and resume
later after a human or external service responds out of band.**
