# Product Roadmap Alignment

> Related:
> - [`index.md`](./index.md)
> - [`runs-and-sessions.md`](./runs-and-sessions.md)
> - [`workspaces.md`](./workspaces.md)
> - [`capability-profiles.md`](./capability-profiles.md)
> - [`out-of-band-approvals.md`](./out-of-band-approvals.md)
> - [`priorities.md`](./priorities.md)
> - [`backlog.md`](./backlog.md)
> - [`../execution/README.md`](../execution/README.md)
> - [`../execution/12-programmable-topology-first-mover.md`](../execution/12-programmable-topology-first-mover.md)
> - [`../execution/13-distributed-runtime-fabric/README.md`](../execution/13-distributed-runtime-fabric/README.md)

## Purpose

This doc connects the product vision in this folder to the execution slices in
`docs/execution/`.

It answers two questions:

1. How do the slices already built map to the product we say we want?
2. How should we choose future slices so they actually deliver against that
   vision rather than becoming disconnected infrastructure work?

## Product Pillars

The product direction in this folder reduces to five pillars:

1. durable sessions
2. reusable conductor extensions
3. provider-neutral runtime fabric
4. portable workspaces
5. portable capability profiles

Every slice should clearly strengthen one of those pillars and ideally unlock a
real end-user or host-product workflow at the same time.

## How Existing Slices Map To The Product

### Foundation and observation

- `01` minimal vertical slice
- `02` hosted runtime
- `06` TS ACP connect

These slices built the basic substrate:

- a hosted ACP runtime exists
- protocol traffic can be observed
- durable state is emitted
- TypeScript can connect to the runtime honestly

Product value delivered:

- Fireline can be a real runtime substrate rather than a design exercise
- durable observation is real

### Runtime lifecycle and launch

- `04` runtime provider lifecycle
- `05` TS host primitive
- `11` agent catalog and runtime launch

These slices built the first runtime-control object:

- `RuntimeDescriptor`
- create/get/list/stop/delete
- catalog-driven launch

Product value delivered:

- a host product can choose and launch agents without hardcoding commands
- runtime creation is no longer just local bootstrap glue

### Durable session center

- `07` durable session catalog and load local
- `08` runtime-owned terminal sessions
- `09` multi-node child-session topology

These slices are the strongest existing alignment with the product vision.

They built:

- durable session rows
- restart-safe session lookup
- runtime-owned session lifetime
- durable child-session topology

Product value delivered:

- Fireline already has the beginnings of a session-first system
- runs are not purely transport-scoped anymore

### Reusable extension layer

- `03` ACP mesh baseline
- `12` programmable topology first mover

These slices are the beginning of the conductor-extension story.

They built or are building:

- peer delegation as a reusable injected capability
- registry-driven optional conductor components
- first proof components such as `audit` and `context_injection`

Product value delivered:

- Fireline can become the reusable extension layer around ACP harnesses rather
  than just a runtime host

## What The Current Slice Set Still Does Not Deliver

The missing pieces map closely to the product gaps:

### 1. Runtime fabric is still under-delivered

The current slices stop at local-first runtime lifecycle.

Missing:

- control-plane-backed discovery
- shared durable-streams deployment across runtimes
- provider-backed remote lifecycle
- runtime auth and registration

This is why slice `13` matters so much.

### 2. Session is durable but not yet a real product object

The data exists, but there is still no clear user-facing or host-product-facing
session surface for:

- list sessions
- reopen sessions
- inspect transcript and artifacts
- manage long-running runs

### 3. Workspace and capability profile do not exist yet

These are the missing objects that translate:

- "my code and files"
- "my MCPs, secrets, skills, and policy"

into portable product concepts.

### 4. Out-of-band approval and credential flows are not yet real

The architecture points there, but the slices do not yet give Fireline a real
story for:

- paused long-running agents
- approval queues
- credential-connection requests
- later resume after a human services the request

## Recommended Alignment Strategy

Do not treat the product docs and slice docs as separate planning systems.

Instead:

1. keep product docs responsible for pillars, user workflows, and object model
2. keep execution docs responsible for proving one narrow technical increment
3. require every new slice to name:
   - the product pillar it strengthens
   - the user workflow it unlocks
   - the proof it delivers

That keeps the slices honest.

## Slice Selection Rule

A new slice should only be accepted if it passes all five checks below.

### 1. Product object check

Which product object does it strengthen?

Examples:

- `Session`
- `Runtime`
- `Workspace`
- `CapabilityProfile`
- `AgentRun`

If the answer is "none", the slice is probably too infrastructural or too
detached from product value.

### 2. User workflow check

Which concrete user or host-product workflow becomes better because of this
slice?

Examples:

- resume a background coding run from the browser
- move a session from local to remote execution
- add GitHub credentials without injecting raw secrets into the runtime
- let a long-running run wait for approval out of band

If the answer is vague, the slice is probably too early.

### 3. Surface check

Which public surface changes?

Examples:

- control-plane API
- TS product API
- runtime bootstrap contract
- conductor component catalog

If a slice has no externally meaningful surface, it should usually be folded
into another slice.

### 4. Durable-evidence check

What durable evidence will exist after the slice lands?

Examples:

- session rows
- child-session edges
- approval requests
- run audit records
- workspace binding records

If the feature matters but leaves no durable evidence, it may not fit the
product direction well.

### 5. Integration check

What adjacent product or workflow can it plug into immediately?

Examples:

- browser harness
- future control plane
- `agent.pw`
- GitHub/Slack workflow backend
- an ACP-native or ACP-adapted agent that Fireline can augment

If it cannot be consumed anywhere, it should have a stronger reason to exist.

## Recommended Next Slice Sequence

Assuming slice `12` is finishing now, the strongest sequence is:

### Phase 0 prerequisite refactor

Strengthens:

- runtime fabric implementation seam

Unlocks:

- a reviewable path for the later control-plane and Docker work without mixing
  mechanical extraction into feature delivery

This is not a product slice by itself. It is a prerequisite refactor.

### `13a` control-plane runtime API and external durable-stream bootstrap

Strengthens:

- runtime fabric
- durable sessions across environment-level runtime boundaries

Unlocks:

- one coherent control-plane-backed local runtime fabric
- browser/control-plane visibility into runtimes whose durable state may live
  outside the runtime process

### `13b` Docker provider and mixed topology

Strengthens:

- runtime fabric
- portable execution placement

Unlocks:

- one coherent local + Docker runtime fabric
- "move this run off the local machine" as a real product direction

### `14` session product surface

Strengthens:

- session as a top-level product object

Unlocks:

- list, inspect, and reopen durable runs through a clean API

This slice should probably be framed around `runs` and `sessions`, not just
another internal state refactor.

### `15` capability profiles and credential references

Strengthens:

- capability profile

Unlocks:

- portable MCP, skills, and policy bundles
- clean `agent.pw` integration through secret or credential-path references

### `16` approval gates and out-of-band service

Strengthens:

- reusable conductor extensions
- long-running background-agent story

Unlocks:

- pause a run on gated action
- service it later through browser/Slack/operator UI
- resume without the original interactive user being present

### `17` workspace model

Strengthens:

- workspace as a product object

Unlocks:

- local path, git ref, or snapshot as first-class run inputs
- a cleaner story for remote execution and later resume

### `18` ACP agent augmentation story

Strengthens:

- Fireline as an upgrade layer around ACP-native or ACP-adapted agents

Unlocks:

- add audit, context injection, approvals, lineage, and durable sessions around
  ACP agents without requiring those agents to build those features natively

## What To Avoid

Avoid slices that are primarily:

- another runtime backend without improving session, workspace, or capability
  portability
- another internal abstraction without a user workflow or product surface
- deeper shared-session machinery before a real consumer appears
- infra work that has no durable evidence and no immediate integration path

Those slices may still be necessary, but they should usually be absorbed into a
more product-legible slice.

## Practical Recommendation

Treat `docs/product/` as the place where Fireline decides what it wants to be.

Treat `docs/execution/` as the place where that vision is broken into narrow
proof-oriented slices.

For each new slice, add one short section near the top:

- `Product Pillar`
- `User Workflow Unlocked`

That small discipline will keep the roadmap aligned without turning every slice
into a PM document.
