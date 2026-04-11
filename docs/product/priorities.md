# Product Priorities

> Related:
> - [`vision.md`](./vision.md)
> - [`object-model.md`](./object-model.md)
> - [`runs-and-sessions.md`](./runs-and-sessions.md)
> - [`workspaces.md`](./workspaces.md)
> - [`capability-profiles.md`](./capability-profiles.md)
> - [`out-of-band-approvals.md`](./out-of-band-approvals.md)
> - [`roadmap-alignment.md`](./roadmap-alignment.md)
> - [`../execution/13-distributed-runtime-fabric/README.md`](../execution/13-distributed-runtime-fabric/README.md)
> - [`../runtime/control-and-data-plane.md`](../runtime/control-and-data-plane.md)
> - [`../state/session-load.md`](../state/session-load.md)

## Purpose

This doc answers a narrower question than the rest of the product folder:

**What surface area must Fireline expose so products like Flamecast can deliver
rich control-plane capabilities without Fireline absorbing that product scope?**

That means this doc is intentionally substrate-first.

It does not assume Fireline should own the full end-user product for:

- agent onboarding
- identity UX
- wallet UX
- service marketplace UX
- demand-side routing UX
- "app store for agents" product packaging

Those are valid product directions for adjacent systems. The question here is
what Fireline must make possible.

## Scope Boundary

Fireline should own:

- runtime lifecycle substrate
- ACP/data-plane entrypoints
- durable session evidence
- canonical read surfaces over durable state
- conductor topology/component seams
- pause/wait/resume mechanics for long-running sessions
- lineage and peer/delegation substrate
- external auth and credential integration seams

Fireline should not try to own:

- the human-facing control-plane product
- the agent identity provider product
- wallet or payment rails
- multi-tenant product databases for every user-facing object
- account provisioning for email/calendar/phone stacks
- demand aggregation or marketplace positioning

The strongest role for Fireline is:

**a durable runtime and extension substrate that exposes clean surfaces for
control planes and products to build on top of.**

## What Fireline Already Has

Fireline is not starting from zero on this direction.

Already present today:

- durable `session`, `runtime_instance`, `prompt_turn`, `permission`, and
  related state rows
- local `session/load` coordination
- runtime-owned session substrate
- lineage-aware peer calls
- programmable runtime topology for optional ACP components
- audit and context-injection as early proof components
- TypeScript-side state materialization over durable streams
- a thin control-plane direction with runtime descriptors, registration, and
  heartbeat

This means the core story is already visible:

- a run can already be durably observed
- runtime behavior can already be modified by reusable conductor components
- the durable stream is already more important than host-local side files
- the control plane can already be thin lifecycle inventory instead of a hot
  path for session traffic

## Distance To The Richer Product Vision

The current stack is uneven in a useful way.

It is already relatively strong on:

- remote/runtime substrate
- durable observation
- ACP-native extension seams
- session durability

It is only partial on:

- out-of-band approvals
- budget/policy enforcement
- portable workspace and capability references
- non-local runtime providers as first-class deployment shapes

It is still early on:

- identity/auth product flows
- delegated budgets and spend controls
- runtime service discovery and routing as a user-facing market
- "basic operating stack" assembly for agents

That is acceptable as long as Fireline keeps its scope on the surfaces that
unlock those later capabilities.

## Product Capability Map

The right way to think about priorities is not "which product object should
Fireline own next?" but:

**which Fireline surface unlocks the largest amount of downstream product
capability?**

| Product capability | Fireline surface that must exist | Current status |
|---|---|---|
| Run an agent remotely and reopen it later from browser or phone | control-plane-backed runtime lifecycle, `RuntimeDescriptor` with advertised `acp`, `state`, and helper endpoints, durable session rows | close on local path, medium overall |
| Trust what the agent did | append-only durable state stream, canonical read schema, audit stream support, replayable lineage | medium |
| Pause when the agent needs approval and resume later | durable permission records, pause/wait/resume semantics, serviceable approval requests | partial |
| Give the agent a rich operating stack | per-session MCP injection, topology component registry, external credential refs, helper file surfaces | partial |
| Delegate to specialist agents or remote hands | peer discovery, child-session edges, lineage propagation, control-plane-backed peer inventory | medium |
| Add budgets and policy without forking the harness | policy component seam, durable budget/approval evidence, external decision service hooks | early |
| Keep auth and secrets out of runtimes | capability/topology refs that point to external credential systems, runtime-call-time header resolution | early |
| Route into services and later marketplaces | runtime catalog, service/tool injection seam, observable usage streams, lineage across calls | early |

The important implication:

- Fireline does not need to ship the full product for these capabilities
- Fireline does need to expose the substrate surfaces that make those products
  straightforward to build

## Fireline Surface Area To Prioritize

These are the highest-value surfaces for Fireline itself.

### 1. Runtime lifecycle and discovery contract

Fireline needs one canonical environment-level runtime contract:

- create/list/get/stop/delete runtime lifecycle
- runtime self-registration and heartbeat
- provider-backed lifecycle for local and non-local runtimes
- `RuntimeDescriptor` with final advertised endpoints
- endpoint-local auth headers or tokens that travel with the descriptor

Why it matters:

- this is the entrypoint for every remote-runtime, browser, mobile, and
  background-agent product flow
- it is the seam that lets Flamecast consume Fireline as substrate rather than
  growing a second runtime stack

### 2. Canonical durable read surface

Fireline needs a stable read contract over durable state:

- canonical row types for runtime, session, prompt turn, permission, terminal,
  chunks, and child-session edges
- replay/catch-up semantics over durable streams
- TypeScript materialization that downstream control planes can embed directly
- a clear distinction between hot ACP traffic and read-oriented state

Why it matters:

- browser, mobile, and operator surfaces should read durable evidence, not
  scrape live runtime state ad hoc
- observability, audit, transcript views, and "reopen this run" all depend on
  this being stable

### 3. Pause / wait / resume surface

Fireline needs a substrate answer for long-running waits:

- durable permission and authorization request records
- explicit paused/waiting state that survives disconnects
- a service path to resolve the wait later
- resume semantics that do not require the original foreground client

Why it matters:

- this is the minimum substrate needed for background agents that can keep
  working after a browser tab closes
- it is the bridge from infrastructure demo to usable workflow

### 4. Extension and policy surface

Fireline needs to keep leaning into conductor composition:

- topology registry with composable proxy and tracer components
- per-session MCP injection
- stable config surfaces for audit, context injection, approval, budget,
  routing, and delegation
- durable evidence that those components can emit or consume

Why it matters:

- this is the clean path to "more agency" without forking every harness
- it keeps Fireline aligned with the ACP SDK's native composition model

### 5. External auth and credential seam

Fireline needs a clear contract for credentials it does not own:

- capability or topology inputs should reference credential ids, paths, or
  scopes rather than raw secrets
- conductor-injected MCP/tool bridges should be able to resolve fresh auth
  headers at call time
- runtimes should avoid becoming credential vaults

Why it matters:

- this is the substrate needed for agent identity/auth stories without turning
  Fireline into `agent.pw`
- it keeps the security boundary compatible with the managed-agents direction:
  session outside the harness, harness outside the sandbox, credentials outside
  generated-code environments

### 6. Portable execution inputs

Fireline needs launch inputs that survive runtime changes:

- stable references for workspace source
- stable references for capability/profile input
- topology and policy defaults that can travel with a run
- helper file APIs and sync strategies hidden behind those references

Why it matters:

- the product layer needs a way to say "same logical work, different hands"
- this is more important than prematurely making `Workspace` or
  `CapabilityProfile` into heavy Fireline-owned product systems

## What This Means For `Session`, `Workspace`, and `CapabilityProfile`

Those concepts are still useful, but the ordering matters.

Near-term, Fireline should prioritize:

- durable session evidence and read interfaces
- stable workspace and capability references in launch contracts
- topology and policy defaults that can travel across runtimes

It should de-emphasize:

- turning each concept into a large Fireline-owned product subsystem too early

In practice:

- a control plane such as Flamecast may expose rich product objects
- Fireline should first expose the substrate surfaces those objects depend on

That keeps Fireline from drifting into product/database scope while still
unlocking those product stories.

## Recommended Slice Ordering

If the goal is to maximize downstream product leverage while keeping Fireline's
scope disciplined, the next priorities should be:

### 1. `13a` distributed runtime fabric first cut

Why first:

- it defines the environment-level runtime contract
- it is the prerequisite for remote, browser, and mobile product flows

What it should prove:

- thin control-plane lifecycle
- runtime descriptors with advertised endpoints
- registration and heartbeat
- external durable-streams as a first-class deployment shape
- no control-plane proxying of ACP/session payloads

### 2. `13b` mixed local + Docker runtime fabric

Why next:

- it is the first real proof that the contract survives beyond local-only mode
- it forces readiness, registration, and shared durable-stream assumptions to
  become real

What it should prove:

- `DockerProvider`
- one coherent mixed-provider environment
- shared durable-streams deployment across runtimes
- control-plane-backed discovery that works for non-local execution

### 3. `14` session product surface, reframed as a Fireline read surface

Why now:

- Fireline already has real durable session substrate
- the missing piece is a stable read contract that downstream products can rely
  on

What it should prove:

- sessions can be listed, inspected, and reopened from durable state
- transcript/history/artifact views derive from stable records
- browser/mobile control planes do not need bespoke runtime scraping

### 4. `16` out-of-band approvals and serviceable waits

Why now:

- it is one of the strongest differentiated workflows unlocked by the substrate
- it is the minimum needed for trustworthy background agents

What it should prove:

- durable pending permission state
- resolution after the original interactive client is gone
- resumed work from durable evidence rather than a held socket

### 5. `15` and `17`, reframed as portable references before rich product objects

Why after the runtime/session surfaces:

- the product layer needs portable inputs
- Fireline does not yet need to own a heavy profile/workspace database model

What they should prove:

- capability refs rather than raw secret injection
- workspace refs rather than ad hoc local-path assumptions
- topology/policy defaults that can travel with a run

### 6. Only then deepen policy, auth, and routing components

Why later:

- these are high leverage only after the underlying runtime, state, and wait
  surfaces are stable

What this should include:

- stronger approval components
- stronger budget components
- richer credential-resolution bridges
- richer service/delegation routing components

## What To De-Emphasize

Do not spend near-term Fireline energy on:

- Fireline-owned wallet or spend systems
- Fireline-owned identity or OAuth UX
- Fireline-owned marketplace or service catalog product
- broad non-ACP protocol adaptation as a primary theme
- building product objects faster than the underlying descriptor/state surfaces
  stabilize

Also avoid building parallel infrastructure in Flamecast that duplicates
Fireline's substrate responsibilities.

The cleaner direction is:

- Fireline exposes runtime, state, pause/resume, and extension surfaces
- Flamecast consumes those surfaces and turns them into the richer product

## Non-Goals

This product direction does not require Fireline to:

- become a full VM or kernel abstraction
- replace MCP
- own the full human-facing control plane
- own the marketplace or demand-side product
- become the credential vault or payment rail
- make remote computer control the primary product abstraction

The goal is narrower and stronger:

**Fireline should expose the durable runtime, state, and extension surfaces that
make rich agent-control-plane products possible without forcing Fireline to
become the whole product.**
