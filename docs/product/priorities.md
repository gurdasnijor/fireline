# Product Priorities

> This doc derives from [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md), which is the operational source of truth for what Fireline builds, in what order, and against what acceptance bars.
>
> Related:
> - [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md) — six-primitive operational source of truth
> - [`../explorations/managed-agents-citations.md`](../explorations/managed-agents-citations.md) — file:line inventory of current implementations
> - [`../execution/13-distributed-runtime-fabric/README.md`](../execution/13-distributed-runtime-fabric/README.md) — runtime fabric umbrella
> - [`../runtime/control-and-data-plane.md`](../runtime/control-and-data-plane.md) — two-plane architecture
> - [`../runtime/heartbeat-and-registration.md`](../runtime/heartbeat-and-registration.md) — push lifecycle reference
>
> Several other docs in this folder (`object-model.md`, `runs-and-sessions.md`, `workspaces.md`, `capability-profiles.md`, `out-of-band-approvals.md`, `product-api-surfaces.md`) predate the substrate-first reframe and are scheduled for rewrite or archival per [`../explorations/doc-staleness-audit.md`](../explorations/doc-staleness-audit.md). Treat them as historical until that cleanup ships.

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

### 3. Orchestration — `wake(session_id)` (currently MISSING)

This is Fireline's largest single primitive gap. The substrate needs a `wake(session_id)` entry point — a scheduler that can call a function with an ID and retry on failure — so that runtimes can be dormant between calls and the durable session log is the source of truth for "where is this run."

What it requires:

- a scheduler service with a `wake(runtime_key, reason)` entry point
- a runtime-side contract for "catch up to durable state on start"
- documented external triggers (webhook ingest, approval resolution, peer call delivery, timer wake-up)
- durable wait records for paused harnesses

Why it matters:

- this is the difference between "agent runs in a tab" and "agent keeps working while you sleep, then notifies you on Slack when it needs approval, then resumes on its own"
- it is the load-bearing dependency for out-of-band approvals, webhook ingestion, queue management, multiplayer driver flows, and harness suspend/resume

See [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md) §"Orchestration" for the full primitive analysis. Owned by slice 18 (new) and slice 16 (out-of-band approvals as the first consumer).

### 4. Harness composition seam (Tools and conductor depth)

Fireline needs to keep leaning into conductor composition:

- topology registry with composable proxy and tracer components
- per-session MCP injection
- stable config surfaces for audit, context injection, approval, budget, routing, and delegation
- durable evidence that those components can emit or consume

Why it matters:

- this is the clean path to "more agency" without forking every harness
- it keeps Fireline aligned with the ACP SDK's native composition model
- the conductor proxy chain is also the suspend/resume seam once Orchestration lands — components that pause mid-effect can persist their continuation through the durable log and resume on `wake`

### 5. Tools — portable references with `credential_ref` indirection

Fireline already has Tools as a strong primitive (MCP injection via topology, `PeerComponent`, `SmitheryComponent`, conductor proxy bridges). The remaining gap is **portable references**:

- launch specs should reference tool bundles or individual tool refs by name, not bake them into spawn arguments
- each tool ref carries a `credential_ref` pointer (secret store path, environment binding, per-session OAuth token)
- credentials resolve at call time, not spawn time
- runtimes never become credential vaults

Why it matters:

- this is the substrate needed for agent identity/auth stories without turning Fireline into `agent.pw`
- it keeps the security boundary compatible with the managed-agents direction: session outside the harness, harness outside the sandbox, credentials outside generated-code environments

Owned by slice 17 (capability profiles, reframed as portable Tools references).

### 6. Resources — `[{source_ref, mount_path}]` (currently MISSING)

Fireline needs launch inputs that survive runtime changes:

- `resources: Vec<ResourceRef>` field on the launch spec
- pluggable `ResourceMounter` implementations for local path, git remote, S3, GCS
- topology and policy defaults that can travel with a run

Why it matters:

- the product layer needs a way to say "same logical work, different hands"
- the previous "Workspace product object" framing was the wrong level of abstraction; the Anthropic Resources primitive collapses it to a launch-spec field with pluggable mounters
- this is dramatically smaller than a product object — a week of refactor work, not a slice

See [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md) §"Resources" for the full primitive analysis. Owned by slice 15 (rewritten as a Resources refactor) with a possible head start from slice 13c (Docker provider needs to mount *something* into containers anyway).

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

The full build order with rationales lives in [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md) §"Build order and slice index". The summary:

| Order | Slice | Primitive | Status |
|---|---|---|---|
| 1 | `13c` Docker provider via bollard | Sandbox (depth) | In flight |
| 2 | `14` Session as canonical read surface | Session (closes the schema gap) | Doc planned, can start in parallel with 13c |
| 3 | `15` Resources refactor (replaces "workspace object") | Resources (closes the gap) | Small refactor, can run in parallel |
| 4 | `18` Orchestration and the `wake` primitive **(NEW)** | Orchestration (closes the gap) | The unblocker — depends on 14 |
| 5 | `16` Out-of-band approvals as a `wake` consumer | Orchestration (first consumer) | Depends on 18 |
| 6 | `17` Capability profiles as portable Tools references | Tools (closes the portability gap) | Independent |
| 7+ | Component depth (richer approval, budget, routing, delegation) | Tools / Harness composition | Ongoing additive work |

### Slice 13a/13b — already shipped

`13a` (control plane runtime API and external durable-stream bootstrap) and `13b` (push lifecycle and bearer auth) shipped in the slice 13 stack. These extend the **Sandbox** primitive and are the foundation everything else builds on.

### Why slice 18 is new

Slice 18 (Orchestration and the `wake` primitive) does not exist in the previous slice plan. It was identified by the managed-agents primitive mapping as the largest single missing primitive. It is sequenced after slice 14 because `wake` needs the canonical read schema to know how to restore a runtime to its last state, and after slice 13c so the cold-start path is exercised against a non-local provider before the scheduler has to rely on it.

### Numbering note

The execution doc filenames `16-capability-profiles.md` and `17-out-of-band-approvals.md` were created in conceptual order (capability profiles first, then approvals) but the build order above puts approvals (slice 16) before capability profiles (slice 17). Two options:

- **Option A: rename files** — `git mv docs/execution/17-out-of-band-approvals.md docs/execution/16-out-of-band-approvals.md` and the reverse for capability profiles. Cleanest long-term. Cost: link updates in five or six docs.
- **Option B: keep filenames stable, document the mismatch** — leave the file names alone and have `managed-agents-mapping.md` be the canonical source of truth for the build order. Cost: ongoing low-level confusion.

This decision is deferred until the slice doc rewrites land (so we can rename and rewrite in the same commit if we go with Option A). The doc audit at [`../explorations/doc-staleness-audit.md`](../explorations/doc-staleness-audit.md) flags this in its "numbering note" section.

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
