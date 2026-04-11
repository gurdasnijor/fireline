# 16: Out-of-Band Approvals as First Orchestration Composition Consumer

Status: planned
Type: execution slice

Related:

- [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md)
- [`../product/out-of-band-approvals.md`](../product/out-of-band-approvals.md)
- [`../product/priorities.md`](../product/priorities.md)
- [`../state/session-load.md`](../state/session-load.md)
- [`../ts/low-level-api-surface.md`](../ts/low-level-api-surface.md)
- [`../explorations/typescript-functional-api-proposal.md`](../explorations/typescript-functional-api-proposal.md)
- [`./14-runs-and-sessions-api.md`](./14-runs-and-sessions-api.md)

## Primitive Anchor

Primitive extended: `Orchestration`

Acceptance-bar items this slice closes:

- at least one worked example of a resumer subscriber loop, with documented
  coordination semantics for multiple concurrent subscribers
- at least one consumer proves the full cycle end-to-end:
  component suspends, event is appended, subscriber sees it, calls
  `resume(sessionId)`, runtime cold-starts if needed, `session/load` rebuilds,
  component releases the pause, agent advances

This slice also closes the remaining Harness suspend/resume items:

- conductor components can pause mid-effect and resume across runtime death by
  writing the pause as an event and rebuilding via `session/load`
- documented contract for what conductor components can do at the
  suspend/resume seam

Depends on:

- slice `14` for canonical Session reads and durable `runtimeSpec` persistence
- the TS-side `resume(sessionId)` helper described in the mapping doc
- the existing `ApprovalGateComponent` and `session/load` substrate

Unblocks:

- later durable waits beyond approvals
- richer approval services built on the same Session + resume pattern
- stronger documentation for component suspend/resume behavior

## Objective

Prove out-of-band approvals as the first worked example of Orchestration by
composition.

This slice does not introduce a wake primitive, a wake endpoint, or a new
control-plane service. It proves the existing composition:

- component writes a durable wait event
- external service subscribes to Session
- external service appends a resolution event
- resumer loop calls `resume(sessionId)`
- runtime cold-starts from stored `runtimeSpec` if needed
- `session/load` rebuilds pending component state from the log
- blocked work advances

The first cut should stay intentionally narrow:

- upgrade `ApprovalGateComponent` to write durable permission events
- upgrade `ApprovalGateComponent` to rebuild pending state from Session on
  `session/load`
- document one resumer subscriber loop
- document coordination semantics for multiple concurrent subscribers

## Product Pillar

Durable orchestration by composition.

## User Workflow Unlocked

A consuming product can pause on a gated tool call, let the runtime go dormant,
service the approval later, and resume the work without depending on a browser
tab or an always-live runtime.

The workflow unlocked by this slice is:

1. agent hits a gated action
2. component writes durable wait state
3. external service or operator resolves the wait later
4. a subscriber observes the resolution and calls `resume(sessionId)`
5. runtime reloads durable state and continues

This is the first concrete proof that Fireline's orchestration story is
composition over Session subscribe + Session append + `session/load` + cold
start, not a new primitive.

## Scope

### 1. Durable approval events, not a product object

This slice should define the durable Session evidence the approval flow needs.

At minimum:

- `PermissionRequest` event on suspend
- `ApprovalResolved` event on later service
- enough identifiers to correlate the resolution back to the blocked effect,
  prompt turn, session, and runtime

This is deliberately narrower than a Fireline-owned `ApprovalRequest` product
API. The substrate proof is about durable wait state on the Session stream.
Higher-level products may later project nicer approval objects on top.

### 2. `ApprovalGateComponent` suspend path

Upgrade `ApprovalGateComponent` so that when it suspends a gated effect it:

- writes `PermissionRequest` as Session evidence
- records the minimum durable correlation data needed for later resolution
- returns a pending state to the harness without assuming the runtime stays
  alive

The important shift is that suspend must become durable by construction rather
than a runtime-local in-memory pause.

### 3. `ApprovalGateComponent` rebuild path

Upgrade `ApprovalGateComponent` so that on `session/load` it can rebuild its
pending approval state from the Session log.

This slice should make explicit:

- how the component finds unresolved `PermissionRequest` entries
- how it matches `ApprovalResolved` entries to those requests
- how it decides whether to release, deny, or keep the pause
- how much of the stream it must scan on rebuild

The success condition is not "the original runtime stayed alive." The success
condition is "a fresh runtime instance can rebuild the gate state from Session
evidence and continue correctly."

### 4. `resume(sessionId)` as the consumer entry point

This slice consumes the TS-side `resume(sessionId)` helper described in
`managed-agents-mapping.md` rather than introducing a new orchestration API.

The approval flow should be documented as:

1. read `session.runtimeSpec` from the Session surface
2. if the runtime is dormant, provision it from that stored spec
3. reconnect ACP
4. call `loadSession(sessionId)`
5. let `ApprovalGateComponent` rebuild from the log and release the pause if a
   matching resolution exists

That dependency on durable `runtimeSpec` persistence is why this slice depends
directly on slice `14`.

### 5. Resumer subscriber worked example

Ship a worked example in docs of the subscriber loop that turns durable Session
events into resumed work.

The example should cover:

- subscribing via `openStream(...)`
- watching for `ApprovalResolved`
- calling `resume(sessionId)`
- handling runtime-not-live by letting `resume` provision from the stored spec
- keeping progress via stream offsets so the subscriber can restart safely

This example is part of the substrate contract. It is how Fireline demonstrates
that "scheduler" means "any subscriber loop that can call `resume(sessionId)`"
rather than "a new service Fireline must invent."

### 6. Multi-subscriber coordination semantics

Document how multiple concurrent subscribers should avoid duplicate resumes.

This slice does not need a full distributed coordination system, but it should
define one workable pattern.

First-cut guidance:

- subscribers read the same Session stream
- before calling `resume(sessionId)`, one subscriber appends a durable claim or
  resume-attempt event keyed by the approval/session
- other subscribers that observe the claim back off
- if the claimer dies, lack of completion plus offset replay lets another
  subscriber retry later

The key property is that coordination itself is durable and stream-driven,
rather than depending on in-memory locks.

### 7. First service path

One real service path should prove the end-to-end loop.

Good candidates:

- browser harness or browser contract test
- a thin control-plane or operator endpoint
- a diagnostic approval-service example

The proof needed is:

- resolution can happen after the original client disconnects
- resolution is written as Session evidence
- a subscriber can observe it and trigger `resume(sessionId)`
- the resumed runtime continues correctly

## Explicit Non-Goals

This slice does **not** require:

- `client.orchestration.*`
- a `wake()` HTTP endpoint
- a Fireline-owned approval queue product
- `client.approvals.*`
- a workflow engine
- complex approval chains
- policy DSL design
- every possible wait type

This is the first worked Orchestration-composition consumer, not a general
approval product.

## Acceptance Criteria

- `ApprovalGateComponent` writes `PermissionRequest` as Session evidence when it
  suspends a gated effect
- `ApprovalGateComponent` can rebuild pending state from the Session log on
  `session/load`
- the flow explicitly depends on slice `14`'s durable `runtimeSpec`
  persistence rather than an external hidden catalog
- a worked resumer subscriber example is documented using `openStream(...)` +
  `resume(sessionId)`
- the docs define coordination semantics for multiple concurrent subscribers
- one end-to-end proof shows:
  - a gated effect suspends
  - the runtime may go dormant
  - a later service appends `ApprovalResolved`
  - a subscriber observes it and calls `resume(sessionId)`
  - the runtime cold-starts if needed
  - `session/load` rebuilds gate state from the log
  - the effect resumes or terminates deterministically

## Validation

- `cargo test -q`
- `pnpm --filter @fireline/client test`
- one conductor/component integration test that:
  - suspends on a gated tool call
  - writes `PermissionRequest`
  - reloads through `session/load`
  - verifies the component rebuilds pending state from the log
- one end-to-end integration test that:
  - resolves the approval after the original client disconnects
  - appends `ApprovalResolved`
  - runs a subscriber loop that calls `resume(sessionId)`
  - verifies the runtime cold-starts from the stored `runtimeSpec` when needed
  - verifies the agent advances after rebuild
- one coordination test or worked example that demonstrates the first-claim
  wins pattern for multiple concurrent subscribers

## Handoff Note

Keep this slice focused on one durable wait pattern.

The handoff should emphasize:

- no new orchestration primitive
- no new control-plane endpoint
- use `resume(sessionId)` as the consumer entry point
- depend on slice `14` for durable `runtimeSpec`
- make `ApprovalGateComponent` durable across runtime death by rebuilding from
  Session on `session/load`
- document multi-subscriber coordination instead of hand-waving it away

This slice proves that Fireline's orchestration story is composition over
existing primitives, not a new scheduler subsystem.
